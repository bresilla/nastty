//! RPC arms in the `auth.*` domain: self-service session methods plus
//! admin user management (upstream-compatible method names).

use nasty_common::{Request, Response};
use serde::Deserialize;

use super::*;
use crate::auth::{Role, Session};
use crate::state::AppState;

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    session: &Session,
) -> Option<Response> {
    Some(match req.method.as_str() {
        "auth.me" => ok(
            req,
            serde_json::json!({
                "username": session.username,
                "role": session.role,
                "must_change_password": session.must_change_password,
            }),
        ),
        "auth.logout" => match state.auth.logout(&session.token).await {
            Ok(()) => ok(req, "ok"),
            Err(e) => err(req, e),
        },
        // Two modes: self-change (requires old_password) and admin reset
        // of another user (upstream's {username, new_password} shape).
        "auth.change_password" => {
            #[derive(Deserialize)]
            struct P {
                username: Option<String>,
                old_password: Option<String>,
                new_password: String,
            }
            match parse_params::<P>(req) {
                Ok(p) => {
                    let target = p.username.as_deref().unwrap_or(&session.username);
                    if target != session.username {
                        if session.role != Role::Admin {
                            return Some(err(req, "admin only"));
                        }
                        match state.auth.admin_set_password(target, &p.new_password).await {
                            Ok(()) => ok(req, "ok"),
                            Err(e) => err(req, e),
                        }
                    } else {
                        match p.old_password {
                            None => invalid(req, "old_password is required"),
                            Some(old) => match state
                                .auth
                                .change_password(&session.username, &old, &p.new_password)
                                .await
                            {
                                Ok(()) => ok(req, "ok"),
                                Err(e) => err(req, e),
                            },
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "auth.list_users" => ok(req, state.auth.list_users().await),
        "auth.create_user" => {
            #[derive(Deserialize)]
            struct P {
                username: String,
                password: String,
                role: Role,
            }
            match parse_params::<P>(req) {
                Ok(p) => match state
                    .auth
                    .create_user(&p.username, &p.password, p.role)
                    .await
                {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "auth.delete_user" => match require_str(req, "username") {
            Ok(username) => match state.auth.delete_user(&session.username, username).await {
                Ok(()) => ok(req, "ok"),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "auth.token.list" => ok(req, state.auth.list_api_tokens().await),
        "auth.token.create" => {
            #[derive(Deserialize)]
            struct P {
                name: String,
                role: Role,
                filesystem: Option<String>,
                expires_in_secs: Option<u64>,
            }
            match parse_params::<P>(req) {
                Ok(p) => match state
                    .auth
                    .create_api_token(&p.name, p.role, p.filesystem, p.expires_in_secs)
                    .await
                {
                    Ok((info, raw)) => ok(
                        req,
                        serde_json::json!({
                            "token": raw,
                            "info": info,
                        }),
                    ),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "auth.token.delete" => match require_str(req, "id") {
            Ok(id) => match state.auth.delete_api_token(id).await {
                Ok(()) => ok(req, "ok"),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        _ => return None,
    })
}
