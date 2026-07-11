# Changelog

## [0.1.1] - 2026-07-11

### <!-- 0 -->⛰️  Features

- Filesystem status view (usage + scrub + fsck)
- Subvolume clone/edit/resize, disk-type override, compact tabs
- NFS client + SMB settings management
- Full iscsi/nvmeof sub-resource mgmt + correct param keys
- SMART health column on Devices tab
- Firewall + notifications in System tab
- Firewall + notifications RPC arms
- Jailed file browser (files.* RPC + Files tab)
- Journal log viewer
- Tuning + NUT editors in System tab
- Device & iscsi/nvmeof sub-resource drill-down
- API tokens tab section with one-time reveal
- API tokens (create/list/delete)
- Dashboard, alerts, system, iscsi/nvmeof, device picker
- Settings, tuning, alerts, metrics, logs, nut, ssh, network arms
- Users, snapshots, share/fs/subvolume management
- User management (list/create/delete/reset)
- Default fs.create to btrfs without bcachefs
- Full parity dispatch and clean N/A errors
- Btrfs backend via forked nasty crates
- Refuse to start without bcachefs-tools
- Startup dependency warnings and help overlay
- Terminal client with live NAS views over JSON-RPC
- NAS API server on upstream nasty crates

### <!-- 1 -->🐛 Bug Fixes

- Show both storage backends, alarm only if none
- Only restore protocols not already running
- Stop starting system services on boot

### <!-- 2 -->🚜 Refactor

- Bcachefs-only on upstream nasty crates

### <!-- 5 -->🎨 Styling

- Two-line card rows, block selection, padded tabs
- Theme, big-text logo, stat tiles, badges

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Changes
- Changes
- Changes
- Devpilot scaffold baseline

### Build

- Bump nasty fork
- Bump nasty fork
- Bump nasty fork to split history
- Add bcachefs/nfs/samba/smart tools to dev shell

