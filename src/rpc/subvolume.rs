//! RPC arms in the `subvolume.*` domain. Ported from the upstream engine's
//! `router/subvolume.rs`; drops the engine-local list_dependents arm.

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
        "subvolume.list_all" => {
            let fs_filter = session.filesystem.as_deref();
            let owner_filter = session.owner.as_deref();
            match state.subvolumes.list_all(fs_filter, owner_filter).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            }
        }
        "subvolume.list" => match require_str(req, "filesystem") {
            Ok(fs_name) => {
                if session.filesystem.as_deref().is_some_and(|p| p != fs_name) {
                    err(req, "access denied")
                } else {
                    match state
                        .subvolumes
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
        "subvolume.get" => match (require_str(req, "filesystem"), require_str(req, "name")) {
            (Ok(fs_name), Ok(name)) => {
                if session.filesystem.as_deref().is_some_and(|p| p != fs_name) {
                    err(req, "access denied")
                } else {
                    match state
                        .subvolumes
                        .get(fs_name, name, session.owner.as_deref())
                        .await
                    {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    }
                }
            }
            (Err(r), _) | (_, Err(r)) => r,
        },
        "subvolume.children" => match (require_str(req, "filesystem"), require_str(req, "name")) {
            (Ok(fs_name), Ok(name)) => match state.subvolumes.list_children(fs_name, name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            (Err(r), _) | (_, Err(r)) => r,
        },
        "subvolume.create" => {
            match parse_params::<nasty_storage::subvolume::CreateSubvolumeRequest>(req) {
                Ok(p) => {
                    if session
                        .filesystem
                        .as_deref()
                        .is_some_and(|f| f != p.filesystem)
                    {
                        err(req, "access denied")
                    } else {
                        let owner = session.owner.clone();
                        match state.subvolumes.create(p, owner).await {
                            Ok(v) => ok(req, v),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "subvolume.delete" => {
            match parse_params::<nasty_storage::subvolume::DeleteSubvolumeRequest>(req) {
                Ok(p) => {
                    if session
                        .filesystem
                        .as_deref()
                        .is_some_and(|f| f != p.filesystem)
                    {
                        err(req, "access denied")
                    } else {
                        match state.subvolumes.delete(p, session.owner.as_deref()).await {
                            Ok(()) => ok(req, "ok"),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "subvolume.attach" => match (require_str(req, "filesystem"), require_str(req, "name")) {
            (Ok(fs_name), Ok(name)) => {
                if session.filesystem.as_deref().is_some_and(|p| p != fs_name) {
                    err(req, "access denied")
                } else {
                    match state
                        .subvolumes
                        .attach(fs_name, name, session.owner.as_deref())
                        .await
                    {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    }
                }
            }
            (Err(r), _) | (_, Err(r)) => r,
        },
        "subvolume.detach" => match (require_str(req, "filesystem"), require_str(req, "name")) {
            (Ok(fs_name), Ok(name)) => {
                if session.filesystem.as_deref().is_some_and(|p| p != fs_name) {
                    err(req, "access denied")
                } else {
                    match state
                        .subvolumes
                        .detach(fs_name, name, session.owner.as_deref())
                        .await
                    {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    }
                }
            }
            (Err(r), _) | (_, Err(r)) => r,
        },
        "subvolume.resize" => {
            match parse_params::<nasty_storage::subvolume::ResizeSubvolumeRequest>(req) {
                Ok(p) => {
                    if session
                        .filesystem
                        .as_deref()
                        .is_some_and(|f| f != p.filesystem)
                    {
                        err(req, "access denied")
                    } else {
                        match state.subvolumes.resize(p, session.owner.as_deref()).await {
                            Ok(v) => ok(req, v),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "subvolume.update" => {
            match parse_params::<nasty_storage::subvolume::UpdateSubvolumeRequest>(req) {
                Ok(p) => {
                    if session
                        .filesystem
                        .as_deref()
                        .is_some_and(|f| f != p.filesystem)
                    {
                        err(req, "access denied")
                    } else {
                        match state.subvolumes.update(p, session.owner.as_deref()).await {
                            Ok(v) => ok(req, v),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "subvolume.clone" => {
            match parse_params::<nasty_storage::subvolume::CloneSubvolumeRequest>(req) {
                Ok(p) => {
                    if session
                        .filesystem
                        .as_deref()
                        .is_some_and(|f| f != p.filesystem)
                    {
                        err(req, "access denied")
                    } else {
                        match state
                            .subvolumes
                            .clone_subvolume(p, session.owner.as_deref())
                            .await
                        {
                            Ok(v) => ok(req, v),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "subvolume.set_properties" => {
            match parse_params::<nasty_storage::subvolume::SetPropertiesRequest>(req) {
                Ok(p) => {
                    if session
                        .filesystem
                        .as_deref()
                        .is_some_and(|sp| sp != p.filesystem)
                    {
                        err(req, "access denied")
                    } else {
                        match state
                            .subvolumes
                            .set_properties(p, session.owner.as_deref())
                            .await
                        {
                            Ok(v) => ok(req, v),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "subvolume.remove_properties" => {
            match parse_params::<nasty_storage::subvolume::RemovePropertiesRequest>(req) {
                Ok(p) => {
                    if session
                        .filesystem
                        .as_deref()
                        .is_some_and(|sp| sp != p.filesystem)
                    {
                        err(req, "access denied")
                    } else {
                        match state
                            .subvolumes
                            .remove_properties(p, session.owner.as_deref())
                            .await
                        {
                            Ok(v) => ok(req, v),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "subvolume.find_by_property" => {
            match parse_params::<nasty_storage::subvolume::FindByPropertyRequest>(req) {
                Ok(p) => {
                    let effective_fs = match (&session.filesystem, &p.filesystem) {
                        (Some(sp), Some(rp)) if sp != rp => {
                            return Some(err(req, "access denied"));
                        }
                        (Some(sp), None) => Some(nasty_storage::subvolume::FindByPropertyRequest {
                            filesystem: Some(sp.clone()),
                            key: p.key.clone(),
                            value: p.value.clone(),
                        }),
                        _ => None,
                    };
                    let req_effective = effective_fs.unwrap_or(p);
                    match state
                        .subvolumes
                        .find_by_property(req_effective, session.owner.as_deref())
                        .await
                    {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        _ => return None,
    })
}
