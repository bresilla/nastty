//! RPC arms in the `auth.*` domain: self-service session methods.

use nasty_common::{Request, Response};
use serde::Deserialize;

use super::*;
use crate::auth::Session;
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
        "auth.change_password" => {
            #[derive(Deserialize)]
            struct P {
                old_password: String,
                new_password: String,
            }
            match parse_params::<P>(req) {
                Ok(p) => match state
                    .auth
                    .change_password(&session.username, &p.old_password, &p.new_password)
                    .await
                {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        _ => return None,
    })
}
