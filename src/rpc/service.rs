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
        "service.protocol.list" => ok(req, protocol_inventory(state).await),
        "service.protocol.enable" => match require_str(req, "name") {
            Ok("rest-server") => err(
                req,
                "rest-server is not part of nastty; `nastty serve` already owns the API",
            ),
            Ok(name) => match ensure_protocol_state_dir().await {
                Ok(()) => match state.protocols.enable(name).await {
                    Ok(v) => ok(req, v),
                    Err(e) => err(req, e),
                },
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "service.protocol.disable" => match require_str(req, "name") {
            Ok("rest-server") => err(
                req,
                "rest-server is not part of nastty; `nastty serve` already owns the API",
            ),
            Ok(name) => match ensure_protocol_state_dir().await {
                Ok(()) => match state.protocols.disable(name).await {
                    Ok(v) => ok(req, v),
                    Err(e) => err(req, e),
                },
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

async fn ensure_protocol_state_dir() -> Result<(), String> {
    tokio::fs::create_dir_all("/var/lib/nasty")
        .await
        .map_err(|error| {
            format!(
                "cannot persist protocol state in /var/lib/nasty: {error}. Start `nastty serve` as root, or run: sudo install -d -m 0755 -o \"$(id -un)\" /var/lib/nasty"
            )
        })
}

async fn protocol_inventory(state: &AppState) -> Vec<serde_json::Value> {
    state
        .protocols
        .list()
        .await
        .into_iter()
        // The upstream rest-server is a separate executable. Nastty has a
        // strict two-binary design, so it is deliberately not exposed.
        .filter(|status| status.name != "rest-server")
        .map(|status| {
            let meta = protocol_metadata(&status.name);
            serde_json::json!({
                "name": status.name,
                "display_name": status.display_name,
                "enabled": status.enabled,
                "running": status.running,
                "system_service": status.system_service,
                "installed": command_exists(meta.binary),
                "package": meta.package,
                "binary": meta.binary,
                "units": meta.units,
                "configuration": meta.configuration,
                "controls": meta.controls,
                "description": meta.description,
            })
        })
        .collect()
}

struct ProtocolMetadata {
    package: &'static str,
    binary: &'static str,
    units: &'static [&'static str],
    configuration: &'static str,
    controls: &'static str,
    description: &'static str,
}

fn protocol_metadata(name: &str) -> ProtocolMetadata {
    match name {
        "nfs" => ProtocolMetadata {
            package: "nfs-kernel-server",
            binary: "exportfs",
            units: &["nfs-server.service"],
            configuration: "/etc/exports.d/nasty.exports",
            controls: "shares, clients, read-only mode, RDMA, nfsd tuning",
            description: "Unix/Linux network filesystem sharing",
        },
        "smb" => ProtocolMetadata {
            package: "samba",
            binary: "smbd",
            units: &[
                "samba-smbd.service",
                "samba-nmbd.service",
                "samba-wsdd.service",
            ],
            configuration: "/etc/samba/smb.nasty.conf",
            controls: "shares, valid users, guest policy, Time Machine, tuning",
            description: "Windows and macOS file sharing",
        },
        "iscsi" => ProtocolMetadata {
            package: "targetcli-fb",
            binary: "targetcli",
            units: &["target.service"],
            configuration: "kernel LIO/configfs",
            controls: "targets, LUNs, portals, ACLs, CHAP",
            description: "SCSI block storage over IP",
        },
        "nvmeof" => ProtocolMetadata {
            package: "nvme-cli",
            binary: "nvme",
            units: &[],
            configuration: "/sys/kernel/config/nvmet",
            controls: "subsystems, namespaces, ports, hosts, TCP/RDMA",
            description: "NVMe block storage over fabrics",
        },
        "nut" => ProtocolMetadata {
            package: "nut",
            binary: "upsmon",
            units: &["nut-monitor.service"],
            configuration: "/etc/nut",
            controls: "UPS mode, driver, monitor target, shutdown policy",
            description: "UPS monitoring and safe shutdown",
        },
        "ssh" => ProtocolMetadata {
            package: "openssh-server",
            binary: "sshd",
            units: &["sshd.service"],
            configuration: "/etc/ssh/sshd_config",
            controls: "password authentication and authorized keys",
            description: "Secure shell access",
        },
        "avahi" => ProtocolMetadata {
            package: "avahi-daemon",
            binary: "avahi-daemon",
            units: &["avahi-daemon.service"],
            configuration: "/etc/avahi/avahi-daemon.conf",
            controls: "mDNS hostname and service discovery",
            description: "Local-network service discovery",
        },
        "smart" => ProtocolMetadata {
            package: "smartmontools",
            binary: "smartctl",
            units: &["smartd.service"],
            configuration: "/etc/smartd.conf",
            controls: "disk health, temperature, tests, warning policy",
            description: "Disk health monitoring",
        },
        _ => ProtocolMetadata {
            package: "system package",
            binary: "",
            units: &[],
            configuration: "system defaults",
            controls: "enable, disable, and inspect status",
            description: "System service",
        },
    }
}

fn command_exists(command: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|path| path.join(command).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_metadata_exposes_installation_and_control_information() {
        let nfs = protocol_metadata("nfs");
        assert_eq!(nfs.package, "nfs-kernel-server");
        assert!(nfs.units.contains(&"nfs-server.service"));
        assert!(nfs.controls.contains("shares"));

        let smart = protocol_metadata("smart");
        assert_eq!(smart.binary, "smartctl");
        assert!(smart.controls.contains("temperature"));
    }
}
