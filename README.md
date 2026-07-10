# nastty

A local NAS built on the [nasty](https://github.com/nasty-project/nasty)
crates: a small API server (`nasttyd`) and a terminal UI (`nastty`), with
**bcachefs and btrfs** storage backends.

The heavy lifting — filesystems, subvolumes, snapshots, NFS/SMB/iSCSI/NVMe-oF
sharing, protocol lifecycle — comes from the `nasty-*` library crates,
consumed from [our fork](https://github.com/bresilla/nasty) (branch `nastty`).
The fork carries only additive changes (the btrfs backend module), so it
rebases onto upstream nearly conflict-free. This repo owns two thin pieces:

- **`nasttyd`** — the server shell:
  - username/password sessions (cookie + bearer token)
  - JSON-RPC 2.0 over WebSocket at `/ws`, with change events pushed to clients
  - REST gateway: `/api/v1/<domain>/<method>` → `<domain>.<method>`
  - upstream-compatible method names (`fs.list`, `device.list`,
    `share.nfs.create`, `service.protocol.enable`, ...)
- **`nastty`** — a [ratatui](https://ratatui.rs) terminal client that logs in,
  speaks the same JSON-RPC over the WebSocket, shows live NAS state in tabs, and
  refreshes automatically on server events.

Not wired (by construction, nothing to strip): VMs, Docker apps, backup,
firmware, Tailscale, OIDC/WebAuthn, web terminal, guest shares, firewall
management.

## Requirements (Ubuntu server)

```sh
# file sharing daemons (NFS + SMB)
sudo apt install -y nfs-kernel-server samba

# one-time state directories (paths are fixed inside the nasty crates)
sudo mkdir -p /var/lib/nasty /fs
sudo chown $USER /var/lib/nasty        # only if running nasttyd unprivileged
```

Optional: `wsdd` (Windows discovery), `smartmontools` (disk health).

Storage backends — at least one is required:

- **btrfs** (the easy path): `sudo apt install btrfs-progs` — the driver is
  already in the Ubuntu kernel. `fs.create` with `"backend": "btrfs"` gives
  you filesystems, subvolumes, and snapshots today.
- **bcachefs**: Ubuntu ships no package; build
  [bcachefs-tools](https://github.com/koverstreet/bcachefs-tools) from source
  plus its out-of-tree kernel module.

Everything else (device listing, shares, protocols) works with either — any
directory under `/fs` can back an NFS/SMB share.

Mutations that touch the system (mkfs, mount, systemctl, exportfs,
smbpasswd) need root; read-only calls degrade gracefully without it.

## Run

```sh
make serve                 # cargo run --bin nasttyd
# or
nasttyd --listen 127.0.0.1:2137
```

**A storage backend is required**: the server refuses to start unless
bcachefs-tools or btrfs-progs is installed, and tells you how to get each.
To develop the API/TUI on a machine that isn't the NAS, pass
`--allow-missing-deps`. At startup a dependency check also warns
(non-fatally) about whichever backend is missing, absent NFS/Samba
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

## Terminal UI

With `nasttyd` running, start the TUI:

```sh
make tui                   # cargo run --bin nastty
# or
nastty --server http://127.0.0.1:2137 --user admin
```

It shows a login screen (and a forced password-change screen on first run),
then a tabbed live view:

- **Overview** — host, kernel, uptime, engine/bcachefs versions
- **Devices** — block devices (`device.list`)
- **Filesystems** — bcachefs filesystems and mount state
- **Subvolumes** — all subvolumes across filesystems
- **Shares** — NFS and SMB shares
- **Protocols** — enable/disable NFS/SMB/iSCSI/NVMe-oF/SSH/mDNS/... with Enter

Keys: `1`–`6` jump to a tab, `←`/`→` cycle tabs, `↑`/`↓` (or `j`/`k`) move the
selection, `Enter` toggles the selected protocol, `r` refreshes, `q` quits. The
view refreshes itself when the server pushes an event, so changes made from
another client show up live.

## Updating to latest upstream

The `nasty-*` dependencies come from the fork
[bresilla/nasty](https://github.com/bresilla/nasty), branch `nastty` —
upstream plus additive commits (the btrfs backend). To pull in new
upstream work:

```sh
cd ../nasty                       # the fork checkout
git fetch upstream
git rebase upstream/main nastty   # additive commits rebase cleanly
git push -f origin nastty
cd ../nastty
cargo update && make verify
```

Fix any small API drift in `src/rpc/` (the handlers are thin ports of
`engine/nasty-engine/src/router/*.rs` — diff those files upstream when
something breaks).

## Development

```sh
make build      # build the library
make test       # run tests
make verify     # fmt-check + check + tests + clippy + rustdoc
make serve      # run the daemon (nasttyd)
make tui        # run the terminal UI (nastty)
```
