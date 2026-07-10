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
    let state = Arc::new(AppState::new().await);

    // Startup restore — trimmed version of the upstream engine's boot
    // sequence. Every phase is non-fatal: a fresh box with an empty state
    // dir sails through all of them.
    let failures = state.filesystems.restore_mounts().await;
    for f in &failures {
        warn!("mount restore failed: {f}");
    }
    let remapped = state.subvolumes.restore_block_devices().await;
    if !remapped.is_empty() {
        info!("restored {} block subvolume device(s)", remapped.len());
    }
    state.protocols.restore().await;
    if let Err(e) = state.smb.ensure_config_scaffolding().await {
        warn!("smb config scaffolding failed (samba not set up?): {e}");
    }

    nastty::server::serve(config.listen, state).await?;
    Ok(())
}
