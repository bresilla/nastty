# nastty

Thin local NAS API server built on the upstream
[nasty](https://github.com/nasty-project/nasty) crates.

All the heavy lifting — bcachefs filesystems, subvolumes, snapshots,
NFS/SMB/iSCSI/NVMe-oF sharing, protocol lifecycle — comes from the upstream
`nasty-*` library crates, consumed as pinned git dependencies. This crate owns
only the server shell:

- username/password sessions (cookie + bearer token)
- JSON-RPC 2.0 over WebSocket at `/ws`, with change events pushed to clients
- REST gateway: `/api/v1/<domain>/<method>` → `<domain>.<method>`
- upstream-compatible method names (`fs.list`, `device.list`,
  `share.nfs.create`, `service.protocol.enable`, ...)

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

bcachefs is only needed for `fs.create` / `fs.mount`: Ubuntu ships no
package, so build [bcachefs-tools](https://github.com/koverstreet/bcachefs-tools)
from source plus its out-of-tree kernel module. Everything else (device
listing, shares, protocols) works without it — any directory under `/fs`
can back an NFS/SMB share.

Mutations that touch the system (mkfs, mount, systemctl, exportfs,
smbpasswd) need root; read-only calls degrade gracefully without it.

## Run

```sh
make serve                 # cargo run --bin nasttyd
# or
nasttyd --listen 127.0.0.1:2137
```

First run creates user `admin` / password `admin` with a forced password
change: every RPC except `auth.change_password` / `auth.me` / `auth.logout`
returns "Password change required" until you change it.

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

## Updating to latest upstream

The `nasty-*` dependencies are pinned to one upstream commit in
`Cargo.toml`. To update:

1. Change the `rev = "..."` on all five `nasty-*` entries to the new
   upstream commit.
2. `cargo update && make verify`.
3. Fix any small API drift in `src/rpc/` (the handlers are thin ports of
   `engine/nasty-engine/src/router/*.rs` — diff those files upstream when
   something breaks).

## Development

```sh
make build      # build the library
make test       # run tests
make verify     # fmt-check + check + tests + clippy + rustdoc
make serve      # run the daemon
```
