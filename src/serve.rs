//! Implementation of the `nastty serve` subcommand.

use std::sync::Arc;

use crate::config::Config;
use crate::state::AppState;
use tracing::{info, warn};

pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let default_filter =
        "nastty=debug,nasty_storage=info,nasty_sharing=info,nasty_snapshot=info,nasty_system=info";
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_filter.into());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("nastty {} serve starting", env!("CARGO_PKG_VERSION"));
    doctor(config.allow_missing_deps)?;
    let state = Arc::new(AppState::new().await);

    // Bind immediately while slow, non-fatal restore work runs in parallel.
    tokio::spawn(restore(state.clone()));
    crate::server::serve(config.listen, state).await?;
    Ok(())
}

/// Verify core NAS dependencies and prepare fixed runtime directories.
fn doctor(allow_missing_deps: bool) -> Result<(), String> {
    let have = |bin: &str| {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|path| path.join(bin).is_file()))
            .unwrap_or(false)
    };

    if !have("bcachefs") {
        if !allow_missing_deps {
            return Err(
                "error: bcachefs-tools is not installed — nastty is a bcachefs NAS and cannot \
                 work without it.\n\
                 Ubuntu ships no package; build it from source:\n\
                 \x20   https://github.com/koverstreet/bcachefs-tools\n\
                 (or use `nastty serve --allow-missing-deps` for development)"
                    .to_string(),
            );
        }
        warn!("bcachefs-tools NOT installed — running in development mode");
    } else {
        let kernel_bcachefs = std::fs::read_to_string("/proc/filesystems")
            .map(|contents| contents.contains("bcachefs"))
            .unwrap_or(false);
        if !kernel_bcachefs {
            warn!(
                "kernel has NO bcachefs support (not in /proc/filesystems) — mounts will fail. \
                 Install the out-of-tree bcachefs module for your kernel"
            );
        }
    }
    if !have("exportfs") {
        warn!(
            "nfs-kernel-server NOT installed — NFS shares unavailable (sudo apt install nfs-kernel-server)"
        );
    }
    if !have("smbd") {
        warn!("samba NOT installed — SMB shares unavailable (sudo apt install samba)");
    }
    ensure_directory(
        "/var/lib/nasty",
        "persistent users, protocol state, and share configuration",
    );
    ensure_directory("/fs", "filesystem mounts and share paths");
    Ok(())
}

fn ensure_directory(path: &str, purpose: &str) {
    if std::path::Path::new(path).is_dir() {
        return;
    }
    match std::fs::create_dir_all(path) {
        Ok(()) => info!("created {path} for {purpose}"),
        Err(error) => warn!(
            "cannot create {path} for {purpose}: {error}. Start `nastty serve` as root, or run: sudo install -d -m 0755 -o \"$(id -un)\" {path}"
        ),
    }
}

async fn restore(state: Arc<AppState>) {
    let failures = state.filesystems.restore_mounts().await;
    for failure in &failures {
        warn!("mount restore failed: {failure}");
    }
    let remapped = state.subvolumes.restore_block_devices().await;
    if !remapped.is_empty() {
        info!("restored {} block subvolume device(s)", remapped.len());
    }
    for protocol in state.protocols.list().await {
        if protocol.name == "rest-server" {
            continue;
        }
        if protocol.enabled && !protocol.running {
            info!(
                "restoring protocol {} (enabled but not running)",
                protocol.name
            );
            match state.protocols.enable(&protocol.name).await {
                Ok(_) => info!("protocol {} restored", protocol.name),
                Err(error) => warn!("protocol {} restore failed: {error}", protocol.name),
            }
        }
    }
    if let Err(error) = state.smb.ensure_config_scaffolding().await {
        warn!("smb config scaffolding failed (samba not set up?): {error}");
    }
    info!("startup restore complete");
}
