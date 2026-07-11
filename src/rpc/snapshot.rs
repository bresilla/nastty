//! RPC arms in the `snapshot.*` domain. Ported from the upstream engine's
//! `router/snapshot.rs`. Rollback deliberately uses the storage primitive
//! directly: nastty has no app/VM lifecycle to quiesce, while the primitive
//! still creates a safety snapshot before swapping the live subvolume.

use nasty_common::{Request, Response};

use super::*;
use crate::auth::Session;
use crate::state::AppState;

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    session: &Session,
) -> Option<Response> {
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
        "snapshot.rollback" => {
            match parse_params::<nasty_storage::subvolume::RollbackSnapshotRequest>(req) {
                Ok(p) => {
                    if session
                        .filesystem
                        .as_deref()
                        .is_some_and(|f| f != p.filesystem)
                    {
                        err(req, "access denied")
                    } else {
                        match state.subvolumes.rollback(p, session.owner.as_deref()).await {
                            Ok(v) => ok(req, v),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        _ => return None,
    })
}
