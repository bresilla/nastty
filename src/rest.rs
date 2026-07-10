//! REST gateway in front of the JSON-RPC dispatcher. Ported from the
//! upstream engine's `rest_gateway.rs`, minus the registry-driven verb
//! check: `/api/v1/foo/bar/baz` maps to method `foo.bar.baz` with params
//! from the query string (GET) or JSON body (POST/PUT/DELETE).

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Map, Value};
use tracing::debug;

use crate::rpc::handle_rpc_request;
use crate::server::token_from_headers;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/api/v1/{*path}",
        get(gateway_get)
            .post(gateway_body)
            .put(gateway_body)
            .delete(gateway_body),
    )
}

async fn gateway_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let params = query_to_json(query.as_deref().unwrap_or(""));
    dispatch(state, headers, &path, params).await
}

async fn gateway_body(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(path): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    dispatch(
        state,
        headers,
        &path,
        body.map(|j| j.0).unwrap_or(Value::Null),
    )
    .await
}

async fn dispatch(
    state: Arc<AppState>,
    headers: HeaderMap,
    path_tail: &str,
    params: Value,
) -> Response {
    let method = path_tail.trim_matches('/').replace('/', ".");

    let client_ip = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local")
        .to_string();
    let Some(token) = token_from_headers(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            json_error("missing session token"),
        )
            .into_response();
    };
    let session = match state.auth.validate(&token, &client_ip).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::UNAUTHORIZED, json_error(e.to_string())).into_response(),
    };

    let request_id = uuid::Uuid::new_v4().to_string();
    let envelope = serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    });
    let raw = serde_json::to_string(&envelope).expect("envelope must serialize");

    debug!("REST→RPC: {} (user: {})", method, session.username);
    let response_str = handle_rpc_request(&raw, &state, &session).await;

    let response: Value = match serde_json::from_str(&response_str) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                json_error(format!("malformed engine response: {e}")),
            )
                .into_response();
        }
    };
    if let Some(err) = response.get("error") {
        let message = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(-32603);
        let status = map_error_to_status(code, message);
        return (
            status,
            Json(serde_json::json!({
                "error": {"code": code, "message": message}
            })),
        )
            .into_response();
    }
    let result = response.get("result").cloned().unwrap_or(Value::Null);
    if matches!(&result, Value::String(s) if s == "ok") || matches!(&result, Value::Null) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::OK, Json(result)).into_response()
    }
}

/// Map JSON-RPC error code + message → HTTP status.
fn map_error_to_status(code: i64, message: &str) -> StatusCode {
    if code == -32700 || code == -32600 || code == -32602 {
        return StatusCode::BAD_REQUEST;
    }
    if code == -32601 {
        return StatusCode::NOT_FOUND;
    }
    let lower = message.to_ascii_lowercase();
    if lower.contains("permission denied") || lower.contains("access denied") {
        return StatusCode::FORBIDDEN;
    }
    if lower.contains("not found") {
        return StatusCode::NOT_FOUND;
    }
    if lower.contains("missing params")
        || lower.contains("missing field")
        || lower.contains("invalid")
    {
        return StatusCode::BAD_REQUEST;
    }
    StatusCode::INTERNAL_SERVER_ERROR
}

/// Parse a URL-encoded query string into a JSON object, treating each value
/// as a JSON literal where it parses (so `?limit=200` becomes
/// `{"limit": 200}`) and as a plain string otherwise.
fn query_to_json(query: &str) -> Value {
    if query.is_empty() {
        return Value::Null;
    }
    let mut obj = Map::new();
    for pair in query.split('&') {
        let (k, v) = match pair.split_once('=') {
            Some(t) => t,
            None => (pair, ""),
        };
        let k = url_decode(k);
        let v = url_decode(v);
        let parsed = serde_json::from_str::<Value>(&v).unwrap_or(Value::String(v));
        obj.insert(k, parsed);
    }
    Value::Object(obj)
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn json_error(msg: impl Into<String>) -> Json<Value> {
    Json(serde_json::json!({"error": msg.into()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_to_json_parses_json_literals() {
        let v = query_to_json("limit=200&name=foo");
        assert_eq!(v, json!({"limit": 200, "name": "foo"}));
    }

    #[test]
    fn query_to_json_url_decodes_values() {
        let v = query_to_json("name=hello%20world&plus=a+b");
        assert_eq!(v, json!({"name": "hello world", "plus": "a b"}));
    }

    #[test]
    fn error_mapping() {
        assert_eq!(
            map_error_to_status(-32603, "Permission denied"),
            StatusCode::FORBIDDEN
        );
        assert_eq!(map_error_to_status(-32601, "x"), StatusCode::NOT_FOUND);
        assert_eq!(map_error_to_status(-32700, "x"), StatusCode::BAD_REQUEST);
        assert_eq!(
            map_error_to_status(-32603, "boom"),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
