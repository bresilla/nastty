//! WebSocket endpoint: JSON-RPC 2.0 request/response plus event
//! notifications. Ported from the upstream engine's `ws_handler` /
//! `handle_socket`.

use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use nasty_common::Notification;
use serde::Deserialize;
use tracing::{debug, info};

use crate::auth::Session;
use crate::rpc::handle_rpc_request;
use crate::server::token_from_headers;
use crate::state::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let client_ip = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local")
        .to_string();
    // Browsers send the session cookie on the upgrade request automatically;
    // resolve it here so the WS task doesn't have to wait for an auth message.
    // Non-browser clients (the TUI) send {"token": "..."} as the first
    // message instead — handled in handle_socket().
    let pre_auth_token = token_from_headers(&headers);
    ws.on_upgrade(move |socket| handle_socket(socket, state, client_ip, pre_auth_token))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    client_ip: String,
    pre_auth_token: Option<String>,
) {
    debug!("WebSocket client connected from {client_ip}, awaiting authentication");

    let mut session = match resolve_session(&mut socket, &state, &client_ip, pre_auth_token).await {
        Some(s) => s,
        None => return,
    };

    info!("WebSocket authenticated as '{}'", session.username);

    let mut event_rx = state.events.subscribe();
    let (mut writer, mut reader) = socket.split();

    // Server-initiated keepalive: ping every 30s, drop clients silent for 90s.
    const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
    const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
    let mut ping_ticker = tokio::time::interval(PING_INTERVAL);
    ping_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ping_ticker.tick().await; // skip the immediate first tick
    let mut last_seen = std::time::Instant::now();

    loop {
        tokio::select! {
            msg = reader.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_seen = std::time::Instant::now();
                        // The session is captured at connect time; while the
                        // password-change gate is active, refresh it so a
                        // successful auth.change_password unblocks this
                        // connection without a reconnect.
                        if session.must_change_password
                            && let Ok(fresh) = state.auth.validate(&session.token, &client_ip).await
                        {
                            session = fresh;
                        }
                        let response = handle_rpc_request(&text, &state, &session).await;
                        if writer.send(Message::Text(response.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Ping(_))) => {
                        last_seen = std::time::Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        debug!("WebSocket read error: {e}");
                        break;
                    }
                    _ => {
                        last_seen = std::time::Instant::now();
                    }
                }
            }
            event = event_rx.recv() => {
                if let Ok(collection) = event {
                    let notification = Notification::new(
                        "event",
                        Some(serde_json::json!({ "collection": collection })),
                    );
                    let text = serde_json::to_string(&notification).unwrap();
                    if writer.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
            _ = ping_ticker.tick() => {
                if last_seen.elapsed() > IDLE_TIMEOUT {
                    let _ = writer.send(Message::Close(None)).await;
                    break;
                }
                if writer.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }

    info!("WebSocket client '{}' disconnected", session.username);
}

/// Pick the right auth path for a WebSocket connection: validate the
/// cookie/Bearer token from the upgrade request when present, otherwise
/// wait for a `{"token": "..."}` first message.
async fn resolve_session(
    socket: &mut WebSocket,
    state: &AppState,
    client_ip: &str,
    pre_auth_token: Option<String>,
) -> Option<Session> {
    if let Some(token) = pre_auth_token {
        return match state.auth.validate(&token, client_ip).await {
            Ok(session) => {
                let _ = socket
                    .send(Message::Text(
                        serde_json::json!({
                            "authenticated": true,
                            "username": session.username,
                            "role": session.role,
                            "must_change_password": session.must_change_password,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
                Some(session)
            }
            Err(_) => {
                let _ = socket
                    .send(Message::Text(r#"{"error":"invalid session"}"#.into()))
                    .await;
                let _ = socket.send(Message::Close(None)).await;
                None
            }
        };
    }
    wait_for_auth(socket, state, client_ip).await
}

/// Wait for the first message, which must be `{"token": "..."}`.
async fn wait_for_auth(
    socket: &mut WebSocket,
    state: &AppState,
    client_ip: &str,
) -> Option<Session> {
    let msg = tokio::time::timeout(std::time::Duration::from_secs(10), socket.recv())
        .await
        .ok()??
        .ok()?;

    let text = match msg {
        Message::Text(t) => t,
        _ => {
            let _ = socket
                .send(Message::Text(
                    r#"{"error":"first message must be JSON with token"}"#.into(),
                ))
                .await;
            return None;
        }
    };

    #[derive(Deserialize)]
    struct AuthMsg {
        token: String,
    }

    let auth_msg: AuthMsg = match serde_json::from_str(&text) {
        Ok(a) => a,
        Err(_) => {
            let _ = socket
                .send(Message::Text(
                    r#"{"error":"expected {\"token\": \"...\"}"}"#.into(),
                ))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            return None;
        }
    };

    match state.auth.validate(&auth_msg.token, client_ip).await {
        Ok(session) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({
                        "authenticated": true,
                        "username": session.username,
                        "role": session.role,
                        "must_change_password": session.must_change_password,
                    })
                    .to_string()
                    .into(),
                ))
                .await;
            Some(session)
        }
        Err(_) => {
            let _ = socket
                .send(Message::Text(r#"{"error":"invalid token"}"#.into()))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            None
        }
    }
}
