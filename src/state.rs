//! Shared application state: the upstream nasty services this server
//! exposes, plus auth and the event bus.

use std::sync::Arc;

/// Broadcast channel for notifying all WebSocket clients of state changes.
/// The payload is the collection name (e.g. "filesystem", "subvolume", "share.nfs").
pub type EventBus = tokio::sync::broadcast::Sender<String>;

pub struct AppState {
    pub auth: crate::auth::AuthService,
    pub events: EventBus,
    /// Whether bcachefs-tools is on PATH. When it isn't, `fs.create`
    /// defaults to the btrfs backend instead.
    pub bcachefs_available: bool,
    /// btrfs-progs version when installed (`btrfs --version`).
    pub btrfs_version: Option<String>,
    pub system: nasty_system::SystemService,
    pub protocols: nasty_system::protocol::ProtocolService,
    pub filesystems: nasty_storage::FilesystemService,
    pub btrfs: nasty_storage::BtrfsService,
    pub subvolumes: Arc<nasty_storage::SubvolumeService>,
    pub snapshots: nasty_snapshot::SnapshotService,
    pub nfs: nasty_sharing::NfsService,
    pub smb: nasty_sharing::SmbService,
    pub iscsi: nasty_sharing::IscsiService,
    pub nvmeof: Arc<nasty_sharing::NvmeofService>,
}

impl AppState {
    pub async fn new() -> Self {
        let (event_tx, _) = tokio::sync::broadcast::channel::<String>(64);
        let subvolumes = Arc::new(nasty_storage::SubvolumeService::new(
            nasty_storage::FilesystemService::new(),
        ));
        let bcachefs_available = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|p| p.join("bcachefs").is_file()))
            .unwrap_or(false);
        // btrfs-progs version, e.g. "6.18" — None when not installed.
        let btrfs_version = nasty_common::cmd::run_ok("btrfs", &["--version"])
            .await
            .ok()
            .and_then(|out| {
                out.split_whitespace()
                    .find(|w| w.starts_with('v'))
                    .map(|v| v.trim_start_matches('v').to_string())
            });
        Self {
            auth: crate::auth::AuthService::new().await,
            events: event_tx,
            bcachefs_available,
            btrfs_version,
            system: nasty_system::SystemService::new(None, None),
            protocols: nasty_system::protocol::ProtocolService::new(),
            filesystems: nasty_storage::FilesystemService::new(),
            btrfs: nasty_storage::BtrfsService::new(),
            snapshots: nasty_snapshot::SnapshotService::new(subvolumes.clone()),
            subvolumes,
            nfs: nasty_sharing::NfsService::new(),
            smb: nasty_sharing::SmbService::new(),
            iscsi: nasty_sharing::IscsiService::new(),
            nvmeof: Arc::new(nasty_sharing::NvmeofService::new()),
        }
    }
}
