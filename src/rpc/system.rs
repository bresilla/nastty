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
        "system.info" => {
            // Upstream's SystemInfo only knows bcachefs; annotate it with
            // the btrfs backend's availability, same as fs.list does.
            let mut val = serde_json::to_value(state.system.info().await).unwrap_or_default();
            if let Some(obj) = val.as_object_mut() {
                obj.insert(
                    "btrfs_version".into(),
                    match &state.btrfs_version {
                        Some(v) => serde_json::Value::String(v.clone()),
                        None => serde_json::Value::Null,
                    },
                );
            }
            ok(req, val)
        }
        "system.health" => ok(req, state.system.health().await),
        _ => return None,
    })
}
