//! RPC arms in the `snapshot.*` domain. Ported from the upstream engine's
//! `router/snapshot.rs`; the engine-orchestrated `snapshot.rollback` is
//! deferred (it quiesces apps/VMs, which this server doesn't manage).

use nasty_common::{Request, Response};

use super::*;
use crate::auth::Session;
use crate::state::AppState;

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    session: &Session,
) -> Option<Response> {
    // btrfs pre-route for filesystem-addressed snapshot ops.
    if let Some(fs_name) = str_param(req, "filesystem")
        && state.btrfs.manages(fs_name).await
    {
        if session.filesystem.as_deref().is_some_and(|p| p != fs_name) {
            return Some(err(req, "access denied"));
        }
        let fs_name = fs_name.to_string();
        return Some(match req.method.as_str() {
            "snapshot.list" => match state.btrfs.snapshot_list(&fs_name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            "snapshot.create" => match (require_str(req, "subvolume"), require_str(req, "name")) {
                (Ok(subvol), Ok(label)) => {
                    match state.btrfs.snapshot_create(&fs_name, subvol, label).await {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    }
                }
                (Err(r), _) | (_, Err(r)) => r,
            },
            "snapshot.delete" => match require_str(req, "name") {
                Ok(name) => match state.btrfs.snapshot_delete(&fs_name, name).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(r) => r,
            },
            "snapshot.clone" => {
                // Mirror the bcachefs param shape {subvolume, snapshot,
                // new_name}; btrfs stores snapshots as `<subvol>@<label>`.
                match (require_str(req, "snapshot"), require_str(req, "new_name")) {
                    (Ok(snapshot), Ok(new_name)) => {
                        let full = if snapshot.contains('@') {
                            snapshot.to_string()
                        } else {
                            match require_str(req, "subvolume") {
                                Ok(sub) => format!("{}@{snapshot}", sub.replace('/', "_")),
                                Err(r) => return Some(r),
                            }
                        };
                        match state.btrfs.snapshot_clone(&fs_name, &full, new_name).await {
                            Ok(v) => ok(req, v),
                            Err(e) => err(req, e),
                        }
                    }
                    (Err(r), _) | (_, Err(r)) => r,
                }
            }
            _ => err(
                req,
                format!("{} is not supported on btrfs filesystems", req.method),
            ),
        });
    }
    Some(match req.method.as_str() {
        "snapshot.list" => match require_str(req, "filesystem") {
            Ok(fs_name) => {
                if session.filesystem.as_deref().is_some_and(|p| p != fs_name) {
                    err(req, "access denied")
                } else {
                    match state
                        .snapshots
                        .list(fs_name, session.owner.as_deref())
                        .await
                    {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    }
                }
            }
            Err(r) => r,
        },
        "snapshot.create" => match parse_params(req) {
            Ok(p) => match state.snapshots.create(p, session.owner.as_deref()).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "snapshot.delete" => match parse_params(req) {
            Ok(p) => match state.snapshots.delete(p, session.owner.as_deref()).await {
                Ok(()) => ok(req, "ok"),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "snapshot.clone" => match parse_params(req) {
            Ok(p) => match state
                .snapshots
                .clone_snapshot(p, session.owner.as_deref())
                .await
            {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        _ => return None,
    })
}
