//! RPC arms in the `alert.*` domain plus `system.alerts`. Ported from
//! the upstream engine's `router/alerts.rs`; the bcachefs deep-health
//! checks (reconcile stall tracking, sysfs error counters) are engine
//! machinery and are skipped — those rule kinds simply never fire here.

use nasty_common::{Request, Response};
use nasty_system::alerts;

use super::*;
use crate::auth::{Role, Session};
use crate::state::AppState;

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    session: &Session,
) -> Option<Response> {
    Some(match req.method.as_str() {
        "system.alerts" => ok(req, evaluate_active_alerts(state).await),
        "alert.rules.list" => ok(req, state.alerts.list_rules().await),
        "alert.rules.create" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match parse_params(req) {
                Ok(rule) => match state.alerts.create_rule(rule).await {
                    Ok(r) => ok(req, r),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "alert.rules.update" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match parse_params::<alerts::AlertRuleUpdate>(req) {
                Ok(update) => match state.alerts.update_rule(&update.id.clone(), update).await {
                    Ok(r) => ok(req, r),
                    Err(e) => err(req, e),
                },
                Err(e) => invalid(req, e),
            }
        }
        "alert.rules.delete" => {
            if session.role != Role::Admin {
                return Some(err(req, "admin only"));
            }
            match require_str(req, "id") {
                Ok(id) => match state.alerts.delete_rule(id).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(r) => r,
            }
        }
        _ => return None,
    })
}

/// Evaluate the configured alert rules against live data. Metrics-daemon
/// absence means CPU/memory/temperature rules can't be judged — return
/// the filesystem-only evaluation rather than fabricating readings.
async fn evaluate_active_alerts(state: &AppState) -> Vec<alerts::ActiveAlert> {
    let stats =
        match fetch_metrics_json::<nasty_system::SystemStats>(&state.metrics_client, "/api/stats")
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!("alert evaluation: stats fetch failed: {e}");
                return Vec::new();
            }
        };

    let filesystems = match state.filesystems.list().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("alert evaluation: filesystems.list failed: {e}");
            Vec::new()
        }
    };
    let mut fs_usage: Vec<alerts::FsUsage> = filesystems
        .iter()
        .map(|p| alerts::FsUsage {
            name: p.name.clone(),
            used_bytes: p.used_bytes,
            total_bytes: p.total_bytes,
        })
        .collect();
    // btrfs filesystems count too.
    if let Ok(btrfs) = state.btrfs.list().await {
        fs_usage.extend(btrfs.into_iter().map(|f| alerts::FsUsage {
            name: f.name,
            used_bytes: f.used_bytes,
            total_bytes: f.total_bytes,
        }));
    }

    let disk_health: Vec<nasty_system::DiskHealth> = if state
        .protocols
        .is_enabled(nasty_system::protocol::Protocol::Smart)
        .await
    {
        fetch_metrics_json(&state.metrics_client, "/api/disks")
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let disk_summary: Vec<alerts::DiskHealthSummary> = disk_health
        .into_iter()
        .map(|d| {
            let critical_attrs_with_value =
                alerts::collect_critical_ata_attrs(&d.smart_status, d.rotational, &d.attributes);
            alerts::DiskHealthSummary {
                device: d.device,
                transport: d.transport,
                temperature_c: d.temperature_c,
                health_passed: d.health_passed,
                smart_status: d.smart_status,
                critical_attrs_with_value,
            }
        })
        .collect();

    let kernel_summary: nasty_common::metrics_types::KernelErrorSummary =
        fetch_metrics_json(&state.metrics_client, "/api/kernel_errors")
            .await
            .unwrap_or_default();
    let kernel_errors = alerts::KernelErrorAlert {
        total_count: kernel_summary.total_count,
        categories: kernel_summary
            .by_category
            .iter()
            .map(|c| c.category.clone())
            .collect(),
    };

    state
        .alerts
        .evaluate(&stats, &fs_usage, &disk_summary, &[], &kernel_errors)
        .await
}
