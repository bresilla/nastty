//! Client for talking to nasttyd: HTTP login plus a JSON-RPC 2.0
//! WebSocket. Shared by the `nastty` TUI and available to any other
//! client (scripts, a future GUI).

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// The connected, authenticated WebSocket stream type.
pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Server acknowledgement sent as the first WS message after auth.
#[derive(Debug, Clone)]
pub struct WsAck {
    pub authenticated: bool,
    pub username: String,
    pub role: String,
    pub must_change_password: bool,
}

/// A message received from the server over the WebSocket.
#[derive(Debug, Clone)]
pub enum Incoming {
    /// Reply to a request we sent, keyed by our request id.
    Response {
        id: i64,
        result: Result<Value, String>,
    },
    /// Server-pushed change event for a collection.
    Event { collection: String },
    /// The post-auth ack or any other JSON we don't model.
    Other(Value),
}

/// Log in over HTTP and return a session token.
pub async fn login(base: &str, username: &str, password: &str) -> Result<String, String> {
    let url = format!("{}/api/login", base.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await
        .map_err(|e| format!("connect to {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(match resp.status().as_u16() {
            401 => "invalid credentials".to_string(),
            code => format!("login failed (HTTP {code})"),
        });
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("parse login response: {e}"))?;
    body.get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "login response missing token".to_string())
}

/// Open the WebSocket, authenticate with the token, and return the
/// stream plus the server's ack.
pub async fn connect_ws(base: &str, token: &str) -> Result<(WsStream, WsAck), String> {
    let ws_url = ws_url(base);
    let (mut stream, _resp) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("websocket connect to {ws_url}: {e}"))?;

    // Non-browser auth path: first message carries the token.
    stream
        .send(Message::Text(
            serde_json::json!({ "token": token }).to_string().into(),
        ))
        .await
        .map_err(|e| format!("send auth message: {e}"))?;

    let ack_text = next_text(&mut stream)
        .await
        .ok_or_else(|| "connection closed before auth ack".to_string())?;
    let ack: Value = serde_json::from_str(&ack_text).map_err(|e| format!("parse auth ack: {e}"))?;
    if let Some(err) = ack.get("error").and_then(|v| v.as_str()) {
        return Err(format!("authentication rejected: {err}"));
    }
    Ok((stream, parse_ack(&ack)))
}

/// Build a JSON-RPC request as a WebSocket text message.
pub fn request(id: i64, method: &str, params: Value) -> Message {
    let envelope = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    Message::Text(envelope.to_string().into())
}

/// Classify a raw WebSocket text frame from the server.
pub fn parse_incoming(text: &str) -> Incoming {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Incoming::Other(Value::String(text.to_string())),
    };

    // Event notification: {"method":"event","params":{"collection":...}}
    if value.get("method").and_then(|v| v.as_str()) == Some("event") {
        let collection = value
            .get("params")
            .and_then(|p| p.get("collection"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        return Incoming::Event { collection };
    }

    // Response envelope: has an id plus result or error.
    if let Some(id) = value.get("id").and_then(|v| v.as_i64()) {
        if let Some(result) = value.get("result") {
            return Incoming::Response {
                id,
                result: Ok(result.clone()),
            };
        }
        if let Some(err) = value.get("error") {
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();
            return Incoming::Response {
                id,
                result: Err(msg),
            };
        }
    }

    Incoming::Other(value)
}

fn parse_ack(ack: &Value) -> WsAck {
    WsAck {
        authenticated: ack
            .get("authenticated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        username: ack
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        role: ack
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        must_change_password: ack
            .get("must_change_password")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

/// Await the next text frame, transparently answering pings and
/// skipping non-text frames.
pub async fn next_text(stream: &mut WsStream) -> Option<String> {
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(t)) => return Some(t.to_string()),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        }
    }
    None
}

fn ws_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    let host = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .unwrap_or(base);
    let scheme = if base.starts_with("https://") {
        "wss"
    } else {
        "ws"
    };
    format!("{scheme}://{host}/ws")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ws_url_derives_from_http_base() {
        assert_eq!(ws_url("http://127.0.0.1:2137"), "ws://127.0.0.1:2137/ws");
        assert_eq!(ws_url("http://127.0.0.1:2137/"), "ws://127.0.0.1:2137/ws");
        assert_eq!(ws_url("https://nas.local"), "wss://nas.local/ws");
    }

    #[test]
    fn parse_incoming_event() {
        match parse_incoming(
            r#"{"jsonrpc":"2.0","method":"event","params":{"collection":"filesystem"}}"#,
        ) {
            Incoming::Event { collection } => assert_eq!(collection, "filesystem"),
            other => panic!("expected event, got {other:?}"),
        }
    }

    #[test]
    fn parse_incoming_ok_response() {
        match parse_incoming(r#"{"jsonrpc":"2.0","id":7,"result":[1,2,3]}"#) {
            Incoming::Response { id, result } => {
                assert_eq!(id, 7);
                assert_eq!(result.unwrap(), json!([1, 2, 3]));
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn parse_incoming_error_response() {
        match parse_incoming(r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32603,"message":"boom"}}"#)
        {
            Incoming::Response { id, result } => {
                assert_eq!(id, 3);
                assert_eq!(result.unwrap_err(), "boom");
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn request_builds_jsonrpc_envelope() {
        let Message::Text(t) = request(1, "fs.list", Value::Null) else {
            panic!("expected text");
        };
        let v: Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 1);
        assert_eq!(v["method"], "fs.list");
    }

    /// Live end-to-end smoke against a running `nasttyd` on the default
    /// port. Ignored by default (needs the server + a fresh admin/admin);
    /// run with `cargo test --lib -- --ignored live_smoke`.
    #[tokio::test]
    #[ignore]
    async fn live_smoke() {
        let base = "http://127.0.0.1:2137";
        let token = login(base, "admin", "admin").await.expect("login");
        let (mut ws, ack) = connect_ws(base, &token).await.expect("connect");
        assert!(ack.authenticated);

        if ack.must_change_password {
            ws.send(request(
                0,
                "auth.change_password",
                json!({"old_password": "admin", "new_password": "hunter2hunter2"}),
            ))
            .await
            .unwrap();
            let text = next_text(&mut ws).await.expect("change_password reply");
            match parse_incoming(&text) {
                Incoming::Response { result, .. } => {
                    assert_eq!(result.unwrap(), json!("ok"))
                }
                other => panic!("unexpected: {other:?}"),
            }
        }

        ws.send(request(1, "device.list", Value::Null))
            .await
            .unwrap();
        let text = next_text(&mut ws).await.expect("device.list reply");
        match parse_incoming(&text) {
            Incoming::Response { id, result } => {
                assert_eq!(id, 1);
                assert!(result.unwrap().is_array());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
