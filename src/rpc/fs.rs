//! RPC arms in the `fs.*` / `device.*` domains. Ported from the upstream
//! engine's `router/fs.rs`; drops the engine-local lock/dependents arms.

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
    // btrfs pre-route: handles btrfs-backed filesystems and the merged
    // fs.list; everything else falls through to the bcachefs arms below.
    if let Some(resp) = btrfs_route(req, state, session).await {
        return Some(resp);
    }
    Some(match req.method.as_str() {
        "fs.get" => match require_str(req, "name") {
            Ok(name) => {
                if session.filesystem.as_deref().is_some_and(|p| p != name) {
                    err(req, "access denied")
                } else {
                    match state.filesystems.get(name).await {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    }
                }
            }
            Err(r) => r,
        },
        "fs.create" => match parse_params(req) {
            Ok(p) => match state.filesystems.create(p).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "fs.destroy" => {
            match parse_params::<nasty_storage::filesystem::DestroyFilesystemRequest>(req) {
                Ok(p) => {
                    if let Some(reason) = check_filesystem_in_use(state, &p.name).await {
                        err(req, reason)
                    } else {
                        match state.filesystems.destroy(p).await {
                            Ok(()) => ok(req, "ok"),
                            Err(e) => err(req, e),
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "fs.mount" => {
            #[derive(Deserialize)]
            struct MountParams {
                name: String,
                #[serde(default)]
                degraded: bool,
            }
            match parse_params::<MountParams>(req) {
                Ok(p) => match state
                    .filesystems
                    .mount_maybe_degraded(&p.name, p.degraded)
                    .await
                {
                    Ok(v) => {
                        // Cascade: restore block devices on this filesystem
                        let _ = state.subvolumes.restore_block_devices().await;
                        ok(req, v)
                    }
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "fs.unmount" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.unmount(name).await {
                Ok(()) => ok(req, "ok"),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.unlock" => match parse_params::<serde_json::Value>(req) {
            Ok(p) => {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let passphrase = p.get("passphrase").and_then(|v| v.as_str()).unwrap_or("");
                match state.filesystems.unlock(name, passphrase).await {
                    Ok(fs) => ok(req, fs),
                    Err(e) => err(req, e),
                }
            }
            Err(e) => invalid(req, e),
        },
        "fs.key.export" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.export_key(name).await {
                Ok(key) => ok(req, key),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.key.delete" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.delete_key(name).await {
                Ok(()) => ok(req, "ok"),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "device.list" => match state.filesystems.list_devices().await {
            Ok(v) => ok(req, v),
            Err(e) => err(req, e),
        },
        "device.set_type" => match parse_params::<nasty_storage::disk_type::DiskTypeUpdate>(req) {
            Ok(u) => match nasty_storage::disk_type::set(u).await {
                Ok(key) => ok(req, serde_json::json!({ "stable_id": key })),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "device.wipe" => match parse_params::<serde_json::Value>(req) {
            Ok(p) => {
                let path = p
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                match state.filesystems.device_wipe(&path).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                }
            }
            Err(e) => invalid(req, e),
        },
        "fs.options.update" => match parse_params(req) {
            Ok(p) => match state.filesystems.update_options(p).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "fs.device.add" => match parse_params(req) {
            Ok(p) => match state.filesystems.device_add(p).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "fs.device.remove" => match parse_params(req) {
            Ok(p) => match state.filesystems.device_remove(p).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "fs.device.evacuate" => {
            match parse_params::<nasty_storage::filesystem::DeviceActionRequest>(req) {
                Ok(p) => {
                    // Validate synchronously before returning
                    match state.filesystems.get(&p.filesystem).await {
                        Err(e) => err(req, e),
                        Ok(fs) if !fs.mounted => err(
                            req,
                            nasty_storage::FilesystemError::CommandFailed(
                                "filesystem must be mounted to evacuate a device".into(),
                            ),
                        ),
                        Ok(_) => {
                            // Run in background — bcachefs evacuate can take many
                            // minutes. Emit filesystem events every 3 s so a UI
                            // shows live device state.
                            let fs_svc = state.filesystems.clone();
                            let events = state.events.clone();
                            tokio::spawn(async move {
                                let poll_events = events.clone();
                                let poll = tokio::spawn(async move {
                                    loop {
                                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                                        let _ = poll_events.send("filesystem".to_string());
                                    }
                                });
                                let _ = fs_svc.device_evacuate(p).await;
                                poll.abort();
                                let _ = events.send("filesystem".to_string());
                            });
                            ok(req, serde_json::json!({"status": "started"}))
                        }
                    }
                }
                Err(e) => invalid(req, e),
            }
        }
        "fs.device.evacuate.cancel" => {
            match parse_params::<nasty_storage::filesystem::DeviceActionRequest>(req) {
                Ok(p) => match state
                    .filesystems
                    .device_evacuate_cancel(&p.filesystem, &p.device)
                    .await
                {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "fs.device.set_state" => match parse_params(req) {
            Ok(p) => match state.filesystems.device_set_state(p).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "fs.device.online" => match parse_params(req) {
            Ok(p) => match state.filesystems.device_online(p).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "fs.device.offline" => match parse_params(req) {
            Ok(p) => match state.filesystems.device_offline(p).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "fs.device.set_label" => match parse_params(req) {
            Ok(p) => match state.filesystems.device_set_label(p).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(e) => invalid(req, e),
        },
        "fs.usage" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.usage(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.scrub.start" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.scrub_start(name).await {
                Ok(()) => ok(req, "ok"),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.scrub.status" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.scrub_status(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.scrub.cancel" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.scrub_cancel(name).await {
                Ok(()) => ok(req, "ok"),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.fsck.start" => {
            #[derive(Deserialize)]
            struct FsckParams {
                name: String,
                #[serde(default)]
                repair: bool,
            }
            match parse_params::<FsckParams>(req) {
                Ok(p) => match state.filesystems.fsck_start(&p.name, p.repair).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "fs.fsck.status" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.fsck_status(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        _ => return None,
    })
}

/// Backend-aware arms. Returns `Some(response)` when the request targets
/// btrfs (or is the merged `fs.list`), `None` to fall through to bcachefs.
async fn btrfs_route(req: &Request, state: &AppState, session: &Session) -> Option<Response> {
    match req.method.as_str() {
        // Merged listing: bcachefs entries (tagged) + btrfs entries.
        "fs.list" => {
            let mut merged: Vec<serde_json::Value> = Vec::new();
            match state.filesystems.list().await {
                Ok(mut v) => {
                    if let Some(ref fs_name) = session.filesystem {
                        v.retain(|p| &p.name == fs_name);
                    }
                    for fs in v {
                        let mut val = serde_json::to_value(fs).unwrap_or_default();
                        if let Some(obj) = val.as_object_mut() {
                            obj.insert("backend".into(), "bcachefs".into());
                        }
                        merged.push(val);
                    }
                }
                Err(e) => return Some(err(req, e)),
            }
            match state.btrfs.list().await {
                Ok(mut v) => {
                    if let Some(ref fs_name) = session.filesystem {
                        v.retain(|p| &p.name == fs_name);
                    }
                    merged.extend(
                        v.into_iter()
                            .map(|fs| serde_json::to_value(fs).unwrap_or_default()),
                    );
                }
                Err(e) => return Some(err(req, e)),
            }
            Some(ok(req, merged))
        }
        // Creation picks the backend from an optional `backend` param.
        "fs.create" if str_param(req, "backend") == Some("btrfs") => {
            match parse_params::<nasty_storage::btrfs::CreateBtrfsRequest>(req) {
                Ok(p) => match state.btrfs.create(p).await {
                    Ok(v) => Some(ok(req, v)),
                    Err(e) => Some(err(req, e)),
                },
                Err(e) => Some(invalid(req, e)),
            }
        }
        // Every other fs.* method addresses a filesystem by `name` (fs.*)
        // or `filesystem` (fs.device.*). When that name is btrfs-managed,
        // handle it here — with an explicit "not supported" for the
        // bcachefs-only concepts, so callers never see a lying
        // "filesystem not found" from the bcachefs service.
        m if m.starts_with("fs.") => {
            let name = str_param(req, "name").or_else(|| str_param(req, "filesystem"))?;
            if session.filesystem.as_deref().is_some_and(|p| p != name) {
                return None; // let the bcachefs arm produce the denial
            }
            if !state.btrfs.manages(name).await {
                return None;
            }
            Some(match req.method.as_str() {
                // A bcachefs create colliding with an existing btrfs name.
                "fs.create" => err(req, format!("filesystem '{name}' already exists (btrfs)")),
                "fs.get" => match state.btrfs.get(name).await {
                    Ok(v) => ok(req, v),
                    Err(e) => err(req, e),
                },
                "fs.mount" => match state.btrfs.mount(name).await {
                    Ok(v) => ok(req, v),
                    Err(e) => err(req, e),
                },
                "fs.unmount" => match state.btrfs.unmount(name).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                "fs.destroy" => {
                    if let Some(reason) = check_filesystem_in_use(state, name).await {
                        err(req, reason)
                    } else {
                        match state.btrfs.destroy(name).await {
                            Ok(()) => ok(req, "ok"),
                            Err(e) => err(req, e),
                        }
                    }
                }
                "fs.usage" => match state.btrfs.usage(name).await {
                    Ok(v) => ok(req, v),
                    Err(e) => err(req, e),
                },
                "fs.scrub.start" => match state.btrfs.scrub_start(name).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                "fs.scrub.status" => match state.btrfs.scrub_status(name).await {
                    Ok(v) => ok(req, v),
                    Err(e) => err(req, e),
                },
                "fs.scrub.cancel" => match state.btrfs.scrub_cancel(name).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                "fs.options.update" => match str_param(req, "compression") {
                    Some(c) => match state.btrfs.update_compression(name, c).await {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    },
                    None => invalid(req, "btrfs supports updating: compression"),
                },
                "fs.device.add" => match require_str(req, "device") {
                    Ok(device) => match state.btrfs.device_add(name, device).await {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    },
                    Err(r) => r,
                },
                "fs.device.remove" => match require_str(req, "device") {
                    Ok(device) => match state.btrfs.device_remove(name, device).await {
                        Ok(v) => ok(req, v),
                        Err(e) => err(req, e),
                    },
                    Err(r) => r,
                },
                // bcachefs-only concepts: encryption/keys, member states,
                // online fsck, per-device labels.
                other => err(
                    req,
                    format!("{other} is not supported on btrfs filesystems"),
                ),
            })
        }
        _ => None,
    }
}
