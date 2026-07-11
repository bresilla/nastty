//! JSON-RPC dispatcher. Ported from the upstream engine's `router/mod.rs`,
//! trimmed to the filesystem / sharing / disk domains this server exposes.

mod alerts;
mod auth;
mod files;
mod fs;
mod service;
mod share;
mod smb;
mod snapshot;
mod subvolume;
mod system;

use nasty_common::{ErrorCode, Request, Response};
use tracing::debug;

use crate::auth::{Role, Session};
use crate::state::AppState;

/// Extract a string param from JSON-RPC params
pub(crate) fn str_param<'a>(request: &'a Request, key: &str) -> Option<&'a str> {
    request
        .params
        .as_ref()
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_str())
}

/// Parse typed params from JSON-RPC request
pub(crate) fn parse_params<T: serde::de::DeserializeOwned>(request: &Request) -> Result<T, String> {
    request
        .params
        .as_ref()
        .ok_or_else(|| "missing params".to_string())
        .and_then(|p| serde_json::from_value(p.clone()).map_err(|e| e.to_string()))
}

#[allow(clippy::result_large_err)]
pub(crate) fn require_str<'a>(req: &'a Request, key: &str) -> Result<&'a str, Response> {
    str_param(req, key).ok_or_else(|| {
        Response::error(
            req.id.clone(),
            ErrorCode::InvalidParams,
            format!("Missing required param: {key}"),
        )
    })
}

pub(crate) fn ok(req: &Request, val: impl serde::Serialize) -> Response {
    Response::success(req.id.clone(), serde_json::to_value(val).unwrap())
}

pub(crate) fn err(req: &Request, e: impl std::fmt::Display) -> Response {
    Response::error(req.id.clone(), ErrorCode::InternalError, e.to_string())
}

pub(crate) fn invalid(req: &Request, msg: impl std::fmt::Display) -> Response {
    Response::error(
        req.id.clone(),
        ErrorCode::InvalidParams,
        format!("Invalid params: {msg}"),
    )
}

/// Return an error response if the given protocol is not enabled.
pub(crate) async fn require_protocol(
    state: &AppState,
    req: &Request,
    proto: nasty_system::protocol::Protocol,
) -> Option<Response> {
    if !state.protocols.is_enabled(proto).await {
        Some(Response::error(
            req.id.clone(),
            ErrorCode::InternalError,
            format!(
                "{} protocol is not enabled — enable it first via service.protocol.enable",
                proto.display_name()
            ),
        ))
    } else {
        None
    }
}

/// Fetch JSON from the (optional) nasty-metrics service.
pub(crate) async fn fetch_metrics_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    path: &str,
) -> Result<T, String> {
    let url = format!("{}{path}", crate::state::METRICS_BASE);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("metrics service unavailable: {e}"))?
        .error_for_status()
        .map_err(|e| format!("metrics service error: {e}"))?;
    resp.json::<T>()
        .await
        .map_err(|e| format!("metrics parse error: {e}"))
}

/// Gate an RDMA-transport request (iSER portal, NVMe-oF rdma port) on
/// the per-box RDMA opt-in, and load the transport's kernel module on
/// the way through.
pub(crate) async fn require_rdma(req: &Request, module: &str) -> Option<Response> {
    if !nasty_system::rdma::enabled().await {
        return Some(Response::error(
            req.id.clone(),
            ErrorCode::InternalError,
            "RDMA transport is disabled on this box",
        ));
    }
    if let Err(e) = nasty_system::rdma::ensure_module(module).await {
        return Some(Response::error(req.id.clone(), ErrorCode::InternalError, e));
    }
    None
}

/// Check if a block device is already exported by another block protocol.
/// Returns an error message if the device is in use, None if it's free.
pub(crate) async fn check_block_device_conflict(
    state: &AppState,
    device_path: &str,
    exclude_protocol: &str,
) -> Option<String> {
    if exclude_protocol != "iscsi"
        && let Ok(targets) = state.iscsi.list().await
    {
        for target in &targets {
            for lun in &target.luns {
                if lun.backstore_path == device_path {
                    return Some(format!(
                        "device {} is already exported via iSCSI (target '{}')",
                        device_path, target.iqn
                    ));
                }
            }
        }
    }
    if exclude_protocol != "nvmeof"
        && let Ok(subsystems) = state.nvmeof.list().await
    {
        for sub in &subsystems {
            for ns in &sub.namespaces {
                if ns.device_path == device_path {
                    return Some(format!(
                        "device {} is already exported via NVMe-oF (subsystem '{}')",
                        device_path, sub.nqn
                    ));
                }
            }
        }
    }
    None
}

/// Refuse to destroy a filesystem that still backs shares or exports.
/// Trimmed version of the engine's dependency walk: checks NFS/SMB share
/// paths under the filesystem mount and iSCSI/NVMe-oF backing devices.
pub(crate) async fn check_filesystem_in_use(state: &AppState, name: &str) -> Option<String> {
    let base = format!("/fs/{name}");
    let under = |p: &str| p == base || p.starts_with(&format!("{base}/"));

    if let Ok(shares) = state.nfs.list().await
        && let Some(s) = shares.iter().find(|s| under(&s.path))
    {
        return Some(format!(
            "filesystem '{name}' is still exported via NFS (share {})",
            s.id
        ));
    }
    if let Ok(shares) = state.smb.list().await
        && let Some(s) = shares.iter().find(|s| under(&s.path))
    {
        return Some(format!(
            "filesystem '{name}' is still shared via SMB (share {})",
            s.id
        ));
    }
    if let Ok(targets) = state.iscsi.list().await
        && targets
            .iter()
            .any(|t| t.luns.iter().any(|l| under(&l.backstore_path)))
    {
        return Some(format!(
            "filesystem '{name}' still backs iSCSI LUNs — remove them first"
        ));
    }
    if let Ok(subsystems) = state.nvmeof.list().await
        && subsystems
            .iter()
            .any(|s| s.namespaces.iter().any(|n| under(&n.device_path)))
    {
        return Some(format!(
            "filesystem '{name}' still backs NVMe-oF namespaces — remove them first"
        ));
    }
    None
}

/// Check if a method is read-only (safe for the ReadOnly role).
fn is_read_only(method: &str) -> bool {
    method.ends_with(".list")
        || method.ends_with(".get")
        || method.ends_with(".status")
        || matches!(
            method,
            "system.info"
                | "system.health"
                | "system.alerts"
                | "system.stats"
                | "system.disks"
                | "system.hardware.summary"
                | "system.metrics.history"
                | "system.metrics.prometheus"
                | "system.network.get"
                | "system.logs"
                | "system.logs.units"
                | "device.list"
                | "auth.me"
                | "files.browse"
                | "fs.usage"
                | "subvolume.list_all"
                | "subvolume.children"
                | "subvolume.find_by_property"
        )
}

/// Self-service methods every authenticated user may call.
fn is_self_service(method: &str) -> bool {
    matches!(method, "auth.logout" | "auth.change_password" | "auth.me")
}

/// Mutations the Operator role may perform (plus everything read-only).
fn is_operator_allowed(method: &str) -> bool {
    is_read_only(method)
        || is_self_service(method)
        || method.starts_with("subvolume.")
        || method.starts_with("snapshot.")
        || method.starts_with("share.")
        || method.starts_with("smb.")
        || method.starts_with("service.protocol.")
        || method.starts_with("files.")
        || matches!(method, "fs.mount" | "fs.unmount")
}

/// Derive the collection name for a mutation method, or None if read-only.
fn collection_for_method(method: &str) -> Option<&'static str> {
    if is_read_only(method) {
        return None;
    }
    match method {
        m if m.starts_with("fs.") => Some("filesystem"),
        m if m.starts_with("device.") => Some("filesystem"),
        m if m.starts_with("subvolume.") => Some("subvolume"),
        m if m.starts_with("snapshot.") => Some("snapshot"),
        m if m.starts_with("share.nfs.") => Some("share.nfs"),
        m if m.starts_with("share.smb.") => Some("share.smb"),
        m if m.starts_with("share.iscsi.") => Some("share.iscsi"),
        m if m.starts_with("share.nvmeof.") => Some("share.nvmeof"),
        m if m.starts_with("service.protocol.") => Some("protocol"),
        _ => None,
    }
}

/// Route a JSON-RPC request to the appropriate handler.
pub async fn handle_rpc_request(raw: &str, state: &AppState, session: &Session) -> String {
    let request: Request = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(_) => {
            let resp = Response::error(
                serde_json::Value::Null,
                ErrorCode::ParseError,
                "Failed to parse JSON-RPC request",
            );
            return serde_json::to_string(&resp).unwrap();
        }
    };

    debug!("RPC call: {} (user: {})", request.method, session.username);

    // Force password change — only allow auth methods until it happens.
    if session.must_change_password && !is_self_service(&request.method) {
        let resp = Response::error(
            request.id,
            ErrorCode::InternalError,
            "Password change required",
        );
        return serde_json::to_string(&resp).unwrap();
    }

    let denied = match session.role {
        Role::Admin => false,
        Role::ReadOnly => !(is_read_only(&request.method) || is_self_service(&request.method)),
        Role::Operator => !is_operator_allowed(&request.method),
    };
    if denied {
        let resp = Response::error(request.id, ErrorCode::InternalError, "Permission denied");
        return serde_json::to_string(&resp).unwrap();
    }

    let t0 = std::time::Instant::now();
    let response = route(&request, state, session).await;
    let elapsed = t0.elapsed();
    if elapsed.as_millis() > 1000 {
        tracing::warn!(
            "RPC slow: {} took {}ms",
            request.method,
            elapsed.as_millis()
        );
    } else {
        debug!("RPC done: {} in {}ms", request.method, elapsed.as_millis());
    }

    // Broadcast event on successful mutations.
    if response.error.is_none()
        && let Some(collection) = collection_for_method(&request.method)
    {
        let _ = state.events.send(collection.to_string());
    }

    serde_json::to_string(&response).unwrap()
}

async fn route(req: &Request, state: &AppState, session: &Session) -> Response {
    let prefix = req
        .method
        .split_once('.')
        .map(|(p, _)| p)
        .unwrap_or(req.method.as_str());
    let resp = match prefix {
        "auth" => auth::try_route(req, state, session).await,
        "alert" => alerts::try_route(req, state, session).await,
        "files" => files::try_route(req, state, session).await,
        "fs" | "device" => fs::try_route(req, state, session).await,
        "subvolume" => subvolume::try_route(req, state, session).await,
        "snapshot" => snapshot::try_route(req, state, session).await,
        "share" => share::try_route(req, state, session).await,
        "smb" => smb::try_route(req, state, session).await,
        "service" => service::try_route(req, state, session).await,
        "system" => {
            // `system.alerts` lives with the alert rules; the rest is system.
            if req.method == "system.alerts" {
                alerts::try_route(req, state, session).await
            } else {
                system::try_route(req, state, session).await
            }
        }
        _ => None,
    };
    resp.unwrap_or_else(|| {
        Response::error(
            req.id.clone(),
            ErrorCode::MethodNotFound,
            format!("Unknown method: {}", req.method),
        )
    })
}
