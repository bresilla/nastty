//! RPC arms in the `system.*` domain (trimmed to info/health).

use nasty_common::{Request, Response};

use super::*;
use crate::auth::Session;
use crate::state::AppState;

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    _session: &Session,
) -> Option<Response> {
    Some(match req.method.as_str() {
        "system.info" => ok(req, state.system.info().await),
        "system.health" => ok(req, state.system.health().await),
        _ => return None,
    })
}
