//! RPC arms in the `smb.*` domain (users/groups). Ported from the upstream
//! engine's `router/smb.rs`.

use nasty_common::{Request, Response};
use serde::Deserialize;

use super::*;
use crate::auth::Session;
use crate::state::AppState;

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    _session: &Session,
) -> Option<Response> {
    Some(match req.method.as_str() {
        "smb.user.list" => match state.smb.list_users().await {
            Ok(v) => ok(req, v),
            Err(e) => err(req, e),
        },
        "smb.user.create" => {
            if let Some(r) =
                require_protocol(state, req, nasty_system::protocol::Protocol::Smb).await
            {
                return Some(r);
            }
            match parse_params::<nasty_sharing::smb::CreateSmbUserRequest>(req) {
                Ok(p) => match state.smb.create_user(p).await {
                    Ok(u) => ok(req, u),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "smb.user.delete" => {
            if let Some(r) =
                require_protocol(state, req, nasty_system::protocol::Protocol::Smb).await
            {
                return Some(r);
            }
            match require_str(req, "username") {
                Ok(username) => match state.smb.delete_user(username).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(r) => r,
            }
        }
        "smb.user.set_password" => {
            if let Some(r) =
                require_protocol(state, req, nasty_system::protocol::Protocol::Smb).await
            {
                return Some(r);
            }
            #[derive(Deserialize)]
            struct P {
                username: String,
                password: String,
            }
            match parse_params::<P>(req) {
                Ok(p) => match state.smb.set_user_password(&p.username, &p.password).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "smb.group.list" => match state.smb.list_groups().await {
            Ok(v) => ok(req, v),
            Err(e) => err(req, e),
        },
        "smb.group.create" => match require_str(req, "name") {
            Ok(name) => match state.smb.create_group(name).await {
                Ok(g) => ok(req, g),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "smb.group.delete" => match require_str(req, "name") {
            Ok(name) => match state.smb.delete_group(name).await {
                Ok(()) => ok(req, "ok"),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "smb.group.add_member" => {
            #[derive(Deserialize)]
            struct P {
                group: String,
                user: String,
            }
            match parse_params::<P>(req) {
                Ok(p) => match state.smb.add_group_member(&p.group, &p.user).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "smb.group.remove_member" => {
            #[derive(Deserialize)]
            struct P {
                group: String,
                user: String,
            }
            match parse_params::<P>(req) {
                Ok(p) => match state.smb.remove_group_member(&p.group, &p.user).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        _ => return None,
    })
}
