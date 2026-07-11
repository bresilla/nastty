//! nastty — thin local NAS API server built on the upstream nasty crates.
//!
//! The heavy lifting (bcachefs, NFS/SMB/iSCSI/NVMe-oF, protocol lifecycle)
//! lives in the `nasty-*` git dependencies; this crate owns only the server
//! shell: auth, JSON-RPC over WebSocket, a REST gateway, and the event bus.

pub mod auth;
pub mod client;
pub mod config;
pub mod metrics;
pub mod rest;
pub mod rpc;
pub mod serve;
pub mod server;
pub mod state;
pub mod tui;
pub mod ws;

/// Returns this crate's display name.
pub fn name() -> &'static str {
    "nastty"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_name() {
        assert_eq!(name(), "nastty");
    }
}
