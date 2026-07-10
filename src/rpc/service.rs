//! RPC arms in the `service.*` domain. Ported from the upstream engine's
//! `router/service.rs`; drops the firewall coupling and the rest-server
//! (backup) arms — nastty manages neither.

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
        "service.protocol.list" => ok(req, state.protocols.list().await),
        "service.protocol.enable" => match require_str(req, "name") {
            Ok(name) => match state.protocols.enable(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "service.protocol.disable" => match require_str(req, "name") {
            Ok(name) => match state.protocols.disable(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "service.base_names.get" => {
            let iqn = tokio::fs::read_to_string("/var/lib/nasty/iscsi-base-iqn")
                .await
                .unwrap_or_else(|_| "iqn.2137-04.storage.nasty".into());
            let nqn = tokio::fs::read_to_string("/var/lib/nasty/nvmeof-base-nqn")
                .await
                .unwrap_or_else(|_| "nqn.2137-04.storage.nasty".into());
            ok(
                req,
                serde_json::json!({ "iqn_prefix": iqn.trim(), "nqn_prefix": nqn.trim() }),
            )
        }
        "service.base_names.update" => {
            if let Some(iqn) = str_param(req, "iqn_prefix")
                && let Err(e) = tokio::fs::write("/var/lib/nasty/iscsi-base-iqn", iqn.trim()).await
            {
                tracing::warn!("persist iscsi base IQN failed: {e}");
            }
            if let Some(nqn) = str_param(req, "nqn_prefix")
                && let Err(e) = tokio::fs::write("/var/lib/nasty/nvmeof-base-nqn", nqn.trim()).await
            {
                tracing::warn!("persist nvmeof base NQN failed: {e}");
            }
            ok(req, "ok")
        }
        _ => return None,
    })
}
