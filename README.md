# nastty

A local **bcachefs** NAS built on the
[nasty](https://github.com/nasty-project/nasty) crates. It builds one
executable, `nastty`, with `serve` and `tui` subcommands.

```sh
nastty serve    # NAS API, persistence, protocols, and built-in metrics
nastty tui      # interactive terminal client
```

The heavy lifting — bcachefs filesystems, subvolumes, snapshots,
NFS/SMB/iSCSI/NVMe-oF sharing, protocol lifecycle — comes from the upstream
`nasty-*` library crates, consumed directly as pinned git dependencies (no
fork). This repo owns one executable with two modes:

- **`nastty serve`** — the server mode:
  - username/password sessions (cookie + bearer token)
  - JSON-RPC 2.0 over WebSocket at `/ws`, with change events pushed to clients
  - REST gateway: `/api/v1/<domain>/<method>` → `<domain>.<method>`
  - built-in CPU, memory, network, and disk-I/O metrics collection, including
    Prometheus output at `/metrics` and in-process history
  - upstream-compatible method names (`fs.list`, `device.list`,
    `share.nfs.create`, `service.protocol.enable`, ...)
- **`nastty tui`** — a [ratatui](https://ratatui.rs) terminal client that logs in,
  speaks the same JSON-RPC over the WebSocket, shows live NAS state in tabs, and
  refreshes automatically on server events.

Not wired (by construction, nothing to strip): VMs, Docker apps, backup,
firmware, Tailscale, OIDC/WebAuthn, web terminal, guest shares, firewall
management.

## Requirements (Ubuntu server)

```sh
# file sharing daemons (NFS + SMB)
sudo apt install -y nfs-kernel-server samba

# only needed before running `nastty serve` as an unprivileged user;
# a root-run server creates these automatically
sudo mkdir -p /var/lib/nasty /fs
sudo chown $USER /var/lib/nasty        # only for an unprivileged server
```

Optional: `wsdd` (Windows discovery), `smartmontools` (disk health).

**bcachefs is required.** Ubuntu ships no package: build
[bcachefs-tools](https://github.com/koverstreet/bcachefs-tools) from source
plus its out-of-tree kernel module. Everything else (device listing, shares,
protocols) still works without it — any directory under `/fs` can back an
NFS/SMB share — but filesystem create/mount needs bcachefs.

Mutations that touch the system (mkfs, mount, systemctl, exportfs,
smbpasswd) need root; read-only calls degrade gracefully without it.

## Run

```sh
make serve                 # cargo run --bin nastty -- serve
# or
nastty serve --listen 127.0.0.1:2137
```

**bcachefs-tools is required**: the server refuses to start without it and
tells you where to get it. To develop the API/TUI on a machine that isn't
the NAS, pass `--allow-missing-deps`. At startup a dependency check also
warns (non-fatally) about missing kernel bcachefs support, absent NFS/Samba
packages, and missing state directories — each with the command to fix it.

First run creates user `admin` / password `admin` with a forced password
change: every RPC except `auth.change_password` / `auth.me` / `auth.logout`
returns "Password change required" until you change it.

Protocol restore at startup is smarter than the upstream appliance's: it
only starts services for protocols that are enabled but **not already
running** (the check is read-only). Services that are already up are left
alone, so an unprivileged `make serve` doesn't trigger a polkit prompt per
service the way a blind `systemctl start` storm would.

## Talk to it

```sh
curl http://127.0.0.1:2137/health
curl http://127.0.0.1:2137/metrics

TOK=$(curl -s -X POST http://127.0.0.1:2137/api/login \
  -H 'content-type: application/json' \
  -d '{"username":"admin","password":"admin"}' | jq -r .token)

curl -X POST -H "Authorization: Bearer $TOK" -H 'content-type: application/json' \
  -d '{"old_password":"admin","new_password":"<something long>"}' \
  http://127.0.0.1:2137/api/v1/auth/change_password

curl -H "Authorization: Bearer $TOK" http://127.0.0.1:2137/api/v1/system/info
curl -H "Authorization: Bearer $TOK" http://127.0.0.1:2137/api/v1/device/list
curl -H "Authorization: Bearer $TOK" http://127.0.0.1:2137/api/v1/fs/list
```

WebSocket (what the TUI uses): connect to `ws://127.0.0.1:2137/ws`, send
`{"token": "..."}` as the first message, then JSON-RPC:

```json
{"jsonrpc": "2.0", "id": 1, "method": "device.list"}
```

After any successful mutation the server pushes a notification on every
connected socket, so clients refresh only the affected collection:

```json
{"jsonrpc": "2.0", "method": "event", "params": {"collection": "filesystem"}}
```

Collections: `filesystem`, `subvolume`, `snapshot`, `share.nfs`,
`share.smb`, `share.iscsi`, `share.nvmeof`, `protocol`.

Metrics are collected inside `nastty serve`; there is no additional metrics
daemon to install or run. The TUI's live dashboard and the `/metrics` endpoint
use the same in-process sampler.

## Terminal UI

With `nastty serve` running, start the TUI:

```sh
make tui                   # cargo run --bin nastty -- tui
# or
nastty tui --server http://127.0.0.1:2137 --user admin
```

It shows a login screen (and a forced password-change screen on first run),
then an interactive workspace organized into five primary sections. On wide
terminals, the current section's views live in a clickable sidebar; compact
terminals keep the same section navigation without sacrificing the workspace.
Selections expose contextual actions instead of acting as read-only dashboard
rows, and `Space` opens their details in an on-demand inspector drawer:

- **Overview** — host, kernel, uptime, engine/bcachefs versions
- **Devices** — block devices (`device.list`)
- **Filesystems** — bcachefs filesystems and mount state
- **Subvolumes** — all subvolumes across filesystems
- **Snapshots** — create, clone, and remove snapshots
- **Shares** — NFS and SMB shares
- **Files** — browse and manage the mounted filesystem tree
- **Protocols** — inspect installation/runtime status and manage
  NFS/SMB/iSCSI/NVMe-oF/SSH/mDNS/... through the Enter control center
- **Users** — accounts, SMB identities, groups, and API tokens
- **Alerts** — active alerts and configurable alert rules
- **System** — host settings, SSH keys, and system logs

Keys: `←`/`→` or `h`/`l` move between the five sections, while
`Tab`/`Shift-Tab` move between the views in the selected section (`[`/`]` are
aliases). `↑`/`↓` or `j`/`k` move the row selection.
`1`–`9`/`0` still jump directly to a view. Press `Space` for the inspector,
`/` or `:` for the searchable command palette, and `?` for the current view's
complete contextual help. The mouse can click section tabs, sidebar views, and
rows or scroll the selection. Forms, confirmations, logs, filesystem status,
resource details, and one-time secrets open as focused windows over the
workspace. Server events refresh the affected collection live.

`Enter` is the primary control action: on a device it opens a device control
center with identity, capacity, SMART data, class management, wipe safety, and
refresh actions. On a protocol/service it opens installation detection,
persistent and runtime state, systemd units, configuration paths, available
tuning areas, enable/disable, and a jump to the related management workspace.
Direct expert shortcuts such as `w`, `t`, and `e` remain available.

The UI runs on Ratatui 0.30 and uses widgets selected from the curated
[`awesome-ratatui`](https://github.com/ratatui/awesome-ratatui) list:

- [`tui-tabs`](https://crates.io/crates/tui-tabs) for the five responsive section tabs
- [`tui-overlay`](https://crates.io/crates/tui-overlay) for the on-demand inspector drawer
- [`tui-popup`](https://github.com/joshka/tui-popup) for the command palette
- [`ratatui-cheese`](https://crates.io/crates/ratatui-cheese) for live spinners and table paginators

## Updating to latest upstream

The `nasty-*` dependencies are pinned to one upstream commit in
`Cargo.toml`. To update:

1. Change the `rev = "..."` on all five `nasty-*` entries to the new
   `nasty-project/nasty` commit.
2. `cargo update && make verify`.
3. Fix any small API drift in `src/rpc/` (the handlers are thin ports of
   `engine/nasty-engine/src/router/*.rs` — diff those files upstream when
   something breaks).

## Development

```sh
make build      # build the single nastty executable
make test       # run tests
make verify     # fmt-check + check + tests + clippy + rustdoc
make serve      # run nastty serve
make tui        # run nastty tui
```

## Release binaries

Publishing a GitHub Release triggers
`.github/workflows/release.yml`. It builds the single `nastty` executable on
native Linux amd64 and arm64 runners, smoke-tests both subcommands, and attaches
a raw executable, `.tar.gz` archive, and SHA-256 checksum manifest for each
architecture:

```text
nastty-linux-amd64
nastty-linux-amd64.tar.gz
nastty-linux-amd64.sha256
nastty-linux-arm64
nastty-linux-arm64.tar.gz
nastty-linux-arm64.sha256
```

Linux is the release platform because the server controls Linux facilities such
as bcachefs, systemd, configfs, NFS, Samba, SMART, and `/proc` metrics.

### Install a released binary

Download the three files for your architecture from the GitHub Release. For
amd64:

```sh
sha256sum -c nastty-linux-amd64.sha256
tar -xzf nastty-linux-amd64.tar.gz
sudo install -m 0755 nastty-linux-amd64 /usr/local/bin/nastty
nastty --version
```

For a 64-bit ARM NAS, replace `amd64` with `arm64`.

The installed file provides both modes—there is no second daemon executable:

```sh
sudo nastty serve
nastty tui
```
