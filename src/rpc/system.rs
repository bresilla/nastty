//! RPC arms in the `system.*` domain. Ported from the upstream engine's
//! `router/system.rs`, trimmed to the NAS surface (no updates/nixos,
//! secure boot, passthrough, tailscale, firewall, or TLS/ACME).

use nasty_common::{Request, Response};

use super::*;
use crate::auth::{Role, Session};
use crate::state::AppState;

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    session: &Session,
) -> Option<Response> {
    Some(match req.method.as_str() {
        "system.info" => ok(req, state.system.info().await),
        "system.health" => ok(req, state.system.health().await),
        "system.hardware.summary" => ok(req, nasty_system::hardware::system_summary().await),

        // ── live metrics (collected inside `nastty serve`) ──────
        "system.stats" => ok(req, state.metrics.stats().await),
        "system.disks" => {
            if state
                .protocols
                .is_enabled(nasty_system::protocol::Protocol::Smart)
                .await
            {
                ok(req, state.metrics.disks().await)
            } else {
                ok(req, Vec::<nasty_system::DiskHealth>::new())
            }
        }
        "system.metrics.history" => {
            let kind = str_param(req, "kind").unwrap_or("net");
            let name = str_param(req, "name");
            let range = str_param(req, "range").unwrap_or("5m");
            let offset = req
                .params
                .as_ref()
                .and_then(|p| p.get("offset"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0);
            ok(req, state.metrics.history(kind, name, range, offset))
        }
        "system.metrics.prometheus" => ok(req, state.metrics.prometheus().await),

        // ── journal logs ────────────────────────────────────────
        "system.logs" => {
            let unit = str_param(req, "unit").unwrap_or("nastty");
            let lines = req
                .params
                .as_ref()
                .and_then(|p| p.get("lines"))
                .and_then(|v| v.as_u64())
                .unwrap_or(100)
                .min(5000)
                .to_string();
            let grep = str_param(req, "grep").filter(|s| !s.is_empty());
            let mut args = vec![
                "-u",
                unit,
                "-n",
                lines.as_str(),
                "--no-pager",
                "--output",
                "short-iso",
            ];
            if let Some(g) = grep {
                args.push("--grep");
                args.push(g);
            }
            match nasty_common::cmd::run_ok("journalctl", &args).await {
                Ok(out) => ok(req, out),
                Err(e) => err(req, e),
            }
        }
        "system.logs.units" => {
            let units = [
                "nastty",
                "nfs-server",
                "smbd",
                "nmbd",
                "sshd",
                "ssh",
                "avahi-daemon",
                "smartd",
                "nut-server",
                "nut-monitor",
            ];
            let mut available = Vec::new();
            for unit in units {
                let svc = format!("{unit}.service");
                if nasty_common::cmd::run_ok("systemctl", &["cat", &svc])
                    .await
                    .is_ok()
                {
                    available.push(unit);
                }
            }
            ok(req, available)
        }

        // ── settings ────────────────────────────────────────────
        "system.settings.get" => ok(req, state.settings.get().await),
        "system.settings.timezones" => match nasty_system::settings::list_timezones().await {
            Ok(v) => ok(req, v),
            Err(e) => err(req, e),
        },
        "system.settings.update" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match parse_params(req) {
                Ok(p) => match state.settings.update(p).await {
                    Ok(v) => ok(req, v),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }

        // ── tuning ──────────────────────────────────────────────
        "system.tuning.get" => ok(req, state.tuning.get().await),
        "system.tuning.update" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match parse_params(req) {
                Ok(p) => match state.tuning.update(p).await {
                    Ok(v) => ok(req, v),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }

        // ── network (read-only; mutations are a later tranche —
        //    upstream's rollback-transaction flow deserves real care) ──
        "system.network.get" => {
            let mgmt = match session.client_ip.as_deref() {
                Some(peer) => nasty_system::network::mgmt_iface_for_peer(peer).await,
                None => None,
            };
            ok(req, state.network.get(mgmt).await)
        }

        // ── UPS (NUT) ───────────────────────────────────────────
        "system.nut.config.get" => ok(req, state.nut.get_config().await.redacted()),
        "system.nut.config.update" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match parse_params(req) {
                Ok(p) => match state.nut.update_config(p).await {
                    Ok(v) => ok(req, v.redacted()),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "system.nut.status" => ok(req, state.nut.status().await),

        // ── firewall ────────────────────────────────────────────
        "system.firewall.status" => ok(req, state.firewall.status().await),
        "system.firewall.restrictions" => ok(req, state.firewall.get_restrictions().await),
        "system.firewall.restrict" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            #[derive(serde::Deserialize)]
            struct P {
                service: String,
                #[serde(default)]
                sources: Vec<String>,
                #[serde(default)]
                interfaces: Vec<String>,
            }
            match parse_params::<P>(req) {
                Ok(p) => match state
                    .firewall
                    .set_restriction(&p.service, p.sources, p.interfaces)
                    .await
                {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }

        // ── notifications (alert delivery) ──────────────────────
        "notifications.config.get" => ok(
            req,
            nasty_system::notifications::NotificationConfig::load().redacted(),
        ),
        "notifications.config.update" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match parse_params::<nasty_system::notifications::NotificationConfig>(req) {
                Ok(config) => match config.apply_update().await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "notifications.test" => {
            match parse_params::<nasty_system::notifications::ChannelType>(req) {
                Ok(channel) => match nasty_system::notifications::test_channel(&channel).await {
                    Ok(msg) => ok(req, msg),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "notifications.test_saved" => match require_str(req, "id") {
            Ok(id) => match nasty_system::notifications::test_saved_channel(id).await {
                Ok(msg) => ok(req, msg),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },

        // ── SSH access ──────────────────────────────────────────
        "system.ssh.status" => {
            let password_auth = tokio::fs::read_to_string("/var/lib/nasty/sshd_override.conf")
                .await
                .unwrap_or_default()
                .contains("yes");
            let keys = tokio::fs::read_to_string("/root/.ssh/authorized_keys")
                .await
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
                .map(|l| l.to_string())
                .collect::<Vec<_>>();
            ok(
                req,
                serde_json::json!({ "password_auth": password_auth, "keys": keys }),
            )
        }
        "system.ssh.add_key" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match require_str(req, "key") {
                Ok(key) => match ssh_add_key(key).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(r) => r,
            }
        }
        "system.ssh.remove_key" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match require_str(req, "key") {
                Ok(key) => match ssh_remove_key(key).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(r) => r,
            }
        }
        _ => return None,
    })
}

const AUTHORIZED_KEYS: &str = "/root/.ssh/authorized_keys";

async fn ssh_add_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if !key.starts_with("ssh-") && !key.starts_with("ecdsa-") {
        return Err("that does not look like an SSH public key".into());
    }
    let mut current = tokio::fs::read_to_string(AUTHORIZED_KEYS)
        .await
        .unwrap_or_default();
    if current.lines().any(|l| l.trim() == key) {
        return Err("key is already present".into());
    }
    if !current.is_empty() && !current.ends_with('\n') {
        current.push('\n');
    }
    current.push_str(key);
    current.push('\n');
    tokio::fs::create_dir_all("/root/.ssh")
        .await
        .map_err(|e| format!("create /root/.ssh: {e}"))?;
    tokio::fs::write(AUTHORIZED_KEYS, current)
        .await
        .map_err(|e| format!("write {AUTHORIZED_KEYS}: {e}"))
}

async fn ssh_remove_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    let current = tokio::fs::read_to_string(AUTHORIZED_KEYS)
        .await
        .map_err(|e| format!("read {AUTHORIZED_KEYS}: {e}"))?;
    let remaining: Vec<&str> = current.lines().filter(|l| l.trim() != key).collect();
    if remaining.len() == current.lines().count() {
        return Err("key not found".into());
    }
    tokio::fs::write(AUTHORIZED_KEYS, remaining.join("\n") + "\n")
        .await
        .map_err(|e| format!("write {AUTHORIZED_KEYS}: {e}"))
}
