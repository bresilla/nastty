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
    if let Err(e) = doctor(config.allow_missing_deps) {
        eprintln!("\n{e}\n");
        std::process::exit(1);
    }
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

/// Startup dependency check. bcachefs-tools is the core of this NAS —
/// without it the server refuses to start (override with
/// `--allow-missing-deps` for API/TUI development on a non-NAS box).
/// Everything else warns with the command to fix it and degrades per
/// capability.
fn doctor(allow_missing_deps: bool) -> Result<(), String> {
    let have = |bin: &str| {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
            .unwrap_or(false)
    };

    // bcachefs-tools is the core of this NAS — refuse to start without it.
    if !have("bcachefs") {
        if !allow_missing_deps {
            return Err(
                "error: bcachefs-tools is not installed — nastty is a bcachefs NAS and cannot \
                 work without it.\n\
                 Ubuntu ships no package; build it from source:\n\
                 \x20   https://github.com/koverstreet/bcachefs-tools\n\
                 (or start with --allow-missing-deps to develop the API/TUI on this machine)"
                    .to_string(),
            );
        }
        warn!("bcachefs-tools NOT installed — running in dev mode (--allow-missing-deps)");
    } else {
        let kernel_bcachefs = std::fs::read_to_string("/proc/filesystems")
            .map(|s| s.contains("bcachefs"))
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
            "cannot create {path} for {purpose}: {error}. Start nasttyd as root, or run: sudo install -d -m 0755 -o \"$(id -un)\" {path}"
        ),
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
        if p.name == "rest-server" {
            continue;
        }
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
