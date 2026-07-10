//! HTTP server: routes, login/logout, session-token plumbing.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use tracing::{info, warn};

use crate::state::AppState;

const SESSION_COOKIE: &str = "nastty_session";
/// Matches the auth session TTL (8 hours).
const COOKIE_MAX_AGE_SECS: u64 = 8 * 3600;

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/api/login", post(login_handler))
        .route("/api/logout", post(logout_handler))
        .route("/api/auth/check", get(auth_check_handler))
        .route("/ws", get(crate::ws::ws_handler))
        .merge(crate::rest::routes())
        .with_state(state)
}

pub async fn serve(
    listen: std::net::SocketAddr,
    state: Arc<AppState>,
) -> Result<(), std::io::Error> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    info!("nasttyd listening on http://{listen}");
    axum::serve(listener, app).await
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "nasttyd",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn login_handler(
    headers: axum::http::HeaderMap,
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let client_ip = client_ip(&headers);
    match state
        .auth
        .login(&req.username, &req.password, client_ip)
        .await
    {
        Ok(token) => {
            info!("login successful: user '{}' from {client_ip}", req.username);
            // Two delivery channels for the same token: Set-Cookie for
            // browsers, JSON body for the TUI and scripts.
            let mut resp_headers = axum::http::HeaderMap::new();
            resp_headers.insert(
                axum::http::header::SET_COOKIE,
                build_session_cookie(&token).parse().unwrap(),
            );
            (
                StatusCode::OK,
                resp_headers,
                Json(serde_json::json!({ "token": token })),
            )
                .into_response()
        }
        Err(_) => {
            warn!("login failed: user '{}' from {client_ip}", req.username);
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "invalid credentials" })),
            )
                .into_response()
        }
    }
}

/// Revoke the current session and clear the cookie.
async fn logout_handler(
    headers: axum::http::HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let mut resp_headers = axum::http::HeaderMap::new();
    resp_headers.insert(
        axum::http::header::SET_COOKIE,
        build_session_clear_cookie().parse().unwrap(),
    );
    if let Some(token) = token_from_headers(&headers) {
        let _ = state.auth.logout(&token).await;
    }
    (
        StatusCode::OK,
        resp_headers,
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

/// Lightweight auth check: 200 if the token is valid, 401 otherwise.
async fn auth_check_handler(
    headers: axum::http::HeaderMap,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let client_ip = client_ip(&headers).to_string();
    match token_from_headers(&headers) {
        Some(token) => match state.auth.validate(&token, &client_ip).await {
            Ok(_) => StatusCode::OK.into_response(),
            Err(_) => (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "invalid token"})),
            )
                .into_response(),
        },
        None => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "missing token"})),
        )
            .into_response(),
    }
}

fn client_ip(headers: &axum::http::HeaderMap) -> &str {
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local")
}

/// Extract the session token: `nastty_session` cookie first, then
/// `Authorization: Bearer`.
pub fn token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(t) = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_session_cookie)
    {
        return Some(t);
    }
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

fn parse_session_cookie(header: &str) -> Option<String> {
    for part in header.split(';') {
        let part = part.trim();
        let (name, value) = match part.split_once('=') {
            Some(t) => t,
            None => continue,
        };
        if name == SESSION_COOKIE && !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn build_session_cookie(token: &str) -> String {
    format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={COOKIE_MAX_AGE_SECS}"
    )
}

fn build_session_clear_cookie() -> String {
    format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
}
