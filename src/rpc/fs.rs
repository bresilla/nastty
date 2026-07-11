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
    Some(match req.method.as_str() {
        "fs.list" => match state.filesystems.list().await {
            Ok(mut v) => {
                if let Some(ref fs_name) = session.filesystem {
                    v.retain(|p| &p.name == fs_name);
                }
                ok(req, v)
            }
            Err(e) => err(req, e),
        },
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
        "fs.lock" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.lock(name).await {
                Ok(fs) => ok(req, fs),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.tpm.status" => match require_str(req, "name") {
            Ok(name) => ok(req, state.filesystems.tpm_status(name).await),
            Err(r) => r,
        },
        "fs.tpm.bind" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.tpm_bind(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.tpm.unbind" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.tpm_unbind(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
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
        "fs.options.update" => match parse_params::<serde_json::Value>(req) {
            Ok(mut raw) => {
                let reconcile = raw
                    .get("reconcile_enabled")
                    .and_then(serde_json::Value::as_bool);
                let copygc = raw
                    .get("copygc_enabled")
                    .and_then(serde_json::Value::as_bool);
                let name = raw
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(obj) = raw.as_object_mut() {
                    obj.remove("reconcile_enabled");
                    obj.remove("copygc_enabled");
                }
                match serde_json::from_value(raw) {
                    Ok(p) => match state.filesystems.update_options(p).await {
                        Ok(v) => {
                            if let Some(enabled) = reconcile
                                && let Err(e) = state
                                    .filesystems
                                    .set_reconcile_enabled(&name, enabled)
                                    .await
                            {
                                return Some(err(req, e));
                            }
                            if let Some(enabled) = copygc
                                && let Err(e) =
                                    state.filesystems.set_copygc_enabled(&name, enabled).await
                            {
                                return Some(err(req, e));
                            }
                            ok(req, v)
                        }
                        Err(e) => err(req, e),
                    },
                    Err(e) => invalid(req, e),
                }
            }
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
        "fs.reconcile.status" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.reconcile_status(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.reconcile.enable" | "fs.reconcile.disable" => match require_str(req, "name") {
            Ok(name) => {
                let enabled = req.method.ends_with(".enable");
                match state.filesystems.set_reconcile_enabled(name, enabled).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                }
            }
            Err(r) => r,
        },
        "fs.copygc.status" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.copygc_status(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "fs.copygc.enable" | "fs.copygc.disable" => match require_str(req, "name") {
            Ok(name) => {
                let enabled = req.method.ends_with(".enable");
                match state.filesystems.set_copygc_enabled(name, enabled).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                }
            }
            Err(r) => r,
        },
        "fs.moving_ctxts" => match require_str(req, "name") {
            Ok(name) => {
                let contexts: Vec<_> = state
                    .filesystems
                    .moving_ctxts(name)
                    .await
                    .into_iter()
                    .map(|ctx| {
                        serde_json::json!({
                            "kind": ctx.kind,
                            "bytes_seen": ctx.bytes_seen,
                            "bytes_moved": ctx.bytes_moved,
                        })
                    })
                    .collect();
                ok(req, contexts)
            }
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
