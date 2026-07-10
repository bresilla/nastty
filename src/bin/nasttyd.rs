//! nasttyd — the nastty API daemon.

use std::sync::Arc;

use nastty::config::{CliAction, parse_args};
use nastty::state::AppState;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let config = match parse_args(&args) {
        Ok(CliAction::Run(c)) => c,
        Ok(CliAction::Exit) => return Ok(()),
        Err(e) => {
            eprintln!("error: {e}\nTry 'nasttyd --help'.");
            std::process::exit(2);
        }
    };

    let default_filter =
        "nastty=debug,nasty_storage=info,nasty_sharing=info,nasty_snapshot=info,nasty_system=info";
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_filter.into());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("nasttyd {} starting", env!("CARGO_PKG_VERSION"));
    doctor();
    let state = Arc::new(AppState::new().await);

    // Startup restore — trimmed version of the upstream engine's boot
    // sequence. Every phase is non-fatal. It runs in the background so the
    // server binds immediately (each phase shells out and can be slow);
    // read-only queries work during restore, and each service is internally
    // synchronised.
    tokio::spawn(restore(state.clone()));

    nastty::server::serve(config.listen, state).await?;
    Ok(())
}

/// Startup dependency check: warn loudly about anything missing that
/// limits functionality, with the command to fix it. Nothing here is
/// fatal — the API works and degrades per capability.
fn doctor() {
    let have = |bin: &str| {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
            .unwrap_or(false)
    };

    if !have("bcachefs") {
        warn!(
            "bcachefs-tools NOT installed — fs.create/mount/unlock will fail. \
             Ubuntu has no package; build from https://github.com/koverstreet/bcachefs-tools"
        );
    }
    let kernel_bcachefs = std::fs::read_to_string("/proc/filesystems")
        .map(|s| s.contains("bcachefs"))
        .unwrap_or(false);
    if !kernel_bcachefs {
        warn!(
            "kernel has NO bcachefs support (not in /proc/filesystems) — mounts will fail. \
             Install the out-of-tree bcachefs module for your kernel"
        );
    }
    if !have("exportfs") {
        warn!(
            "nfs-kernel-server NOT installed — NFS shares unavailable (sudo apt install nfs-kernel-server)"
        );
    }
    if !have("smbd") {
        warn!("samba NOT installed — SMB shares unavailable (sudo apt install samba)");
    }
    if !std::path::Path::new("/var/lib/nasty").is_dir() {
        warn!(
            "/var/lib/nasty does not exist — users, protocol state, and share configs will NOT \
             persist across restarts (sudo mkdir -p /var/lib/nasty && sudo chown $USER /var/lib/nasty)"
        );
    }
    if !std::path::Path::new("/fs").is_dir() {
        warn!(
            "/fs does not exist — filesystem mounts and share paths live there (sudo mkdir -p /fs)"
        );
    }
}

async fn restore(state: Arc<AppState>) {
    let failures = state.filesystems.restore_mounts().await;
    for f in &failures {
        warn!("mount restore failed: {f}");
    }
    let remapped = state.subvolumes.restore_block_devices().await;
    if !remapped.is_empty() {
        info!("restored {} block subvolume device(s)", remapped.len());
    }
    // Protocol restore, but smarter than upstream's: upstream re-asserts
    // every enabled protocol with `systemctl start` unconditionally, which
    // triggers a polkit auth prompt per service when running unprivileged —
    // even for services that are already up. Only touch protocols that are
    // enabled but NOT running (the running check is read-only, no auth).
    for p in state.protocols.list().await {
        if p.enabled && !p.running {
            info!("restoring protocol {} (enabled but not running)", p.name);
            match state.protocols.enable(&p.name).await {
                Ok(_) => info!("protocol {} restored", p.name),
                Err(e) => warn!("protocol {} restore failed: {e}", p.name),
            }
        }
    }
    if let Err(e) = state.smb.ensure_config_scaffolding().await {
        warn!("smb config scaffolding failed (samba not set up?): {e}");
    }
    info!("startup restore complete");
}
