# NASty upstream findings for `nastty`

Source inspected: `https://github.com/nasty-project/nasty`

- Local inspection clone: `/tmp/nasty-upstream`
- Upstream commit inspected: `4ad3fa304b57b8ce82774a36693aa87769d1478b`
- Upstream commit date: `2026-07-09T09:57:15+02:00`
- Upstream version in `engine/Cargo.toml`: `0.0.13`

This document focuses on the API/server architecture only. Reverse proxies,
TLS, Caddy, and the appliance installer are not the important pieces for the
first `nastty` design.

## Short answer

Yes: NASty already has a real local API server.

The actual backend server is:

```text
engine/nasty-engine/src/main.rs
```

It is an Axum server that binds directly to:

```text
127.0.0.1:2137
```

The WebUI is just a client. It talks to that local server mostly through
JSON-RPC 2.0 over WebSocket.

For `nastty`, the useful first architecture is:

```text
CLI / local UI / scripts
  -> http://127.0.0.1:2137
  -> nastty API server
  -> storage / sharing / system modules
```

No reverse proxy is needed for the local-only version.

## Upstream project shape

Top-level upstream structure:

```text
engine/         Rust workspace: backend server and modules
webui/          SvelteKit frontend
nixos/          NixOS appliance modules and systemd units
vendor/         Swagger UI assets
docs/           docs and plans
```

The Rust backend workspace members are:

```text
nasty-engine    API server, auth, routing, file handlers, websocket handlers
nasty-common    JSON-RPC types, command helpers, state helpers, secrets
nasty-metrics   metrics daemon on port 2138
nasty-storage   bcachefs filesystem and subvolume management
nasty-sharing   NFS, SMB, iSCSI, NVMe-oF
nasty-snapshot  snapshot wrapper layer
nasty-system    network, updates, settings, protocol lifecycle, firewall, etc.
nasty-vm        QEMU/KVM VM management
nasty-apps      Docker/Compose apps
nasty-backup    backup profiles and scheduler
```

For a modular NAS server, the key reusable pieces are:

```text
nasty-engine
nasty-common
nasty-storage
nasty-sharing
nasty-system, but probably trimmed
nasty-metrics, optional
```

The pieces to defer or drop at first:

```text
nasty-vm
nasty-apps
nasty-backup
OIDC/WebAuthn if you want simpler auth first
web terminal
guest share public links
firmware updates
tailscale
secure boot ceremony
```

## How the server starts

In upstream, systemd runs:

```text
nasty-engine
```

with:

```text
ExecStart = .../bin/nasty-engine
```

Inside `engine/nasty-engine/src/main.rs`, startup does roughly this:

1. Parse special CLI modes:
   - `--version`
   - `bootstrap-system-flake`
   - `--dump-docs`
2. Configure tracing/logging.
3. Create an event bus:
   - `tokio::sync::broadcast::channel::<String>(64)`
4. Build shared `AppState`.
5. Restore persisted system state.
6. Start background jobs.
7. Mark boot status ready.
8. Notify systemd ready.
9. Register Axum HTTP/WebSocket routes.
10. Bind to `127.0.0.1:2137`.

The important `AppState` fields are:

```text
auth
events
system
settings
network
protocols
firewall
filesystems
subvolumes
snapshots
nfs
smb
iscsi
nvmeof
vms
apps
backups
metrics_client
```

For `nastty`, this `AppState` should be split into smaller modules instead of
keeping one giant kitchen-sink state object.

## Startup restoration sequence

Before accepting normal API calls, upstream restores state from disk:

```text
filesystems.restore_mounts
subvolumes.restore_block_devices
nvmeof.remap_device_paths
iscsi.remap_device_paths
protocols.restore
smb.scaffold_config
domain.restore
nvmeof.restore
vms.restore
apps.restore
tailscale.restore
network.restore_pending_revert
network.reconcile_orphans
subvolumes.reconcile_project_ids
apps.reconcile_app_routes
apps.reconcile_networks
backups.migrate_secrets
nut.migrate_secrets
oidc.migrate_secrets
iscsi.migrate_secrets
notifications.migrate_secrets
firewall.init
nvmeof.ensure_tailscale_ports
caches.warm
```

For a simpler local-only `nastty`, the first useful startup sequence is much
smaller:

```text
auth.load
storage.restore_mounts
subvolumes.restore_block_devices
protocols.restore
smb.scaffold_config
firewall.init, optional
serve API
```

## How the WebUI talks to the server

Frontend singleton client:

```text
webui/src/lib/client.ts
```

It creates:

```ts
new NastyClient(`${wsProto}//${host}/ws`)
```

RPC client:

```text
webui/src/lib/rpc.ts
```

It sends JSON-RPC 2.0 messages over WebSocket:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "system.info",
  "params": {}
}
```

Server replies:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {}
}
```

or:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32603,
    "message": "..."
  }
}
```

The GUI does not call Rust directly. It is only an API client.

## Auth flow

Browser login:

```http
POST /api/login
Content-Type: application/json

{
  "username": "admin",
  "password": "admin"
}
```

Server response includes:

- JSON body with token
- `Set-Cookie: nasty_session=...`

The WebUI uses the cookie. CLI clients can use the token.

Token extraction order in the server:

1. `nasty_session` cookie
2. `Authorization: Bearer ...`

For WebSocket clients:

- Browser path: cookie is sent automatically on `/ws`.
- Non-browser path: first WebSocket message can be:

```json
{ "token": "..." }
```

Then normal JSON-RPC calls follow.

Fresh install behavior:

- If `/var/lib/nasty/auth.json` is uninitialized, upstream creates:

```text
username: admin
password: admin
role: admin
must_change_password: true
```

For `nastty`, this should probably be changed to a safer first-run setup or
explicit bootstrap command.

## Main server routes

The local server registers these important routes:

```text
GET  /health
POST /api/login
POST /api/logout
GET  /api/auth/check
GET  /api/boot_status
GET  /api/openapi.json
ANY  /api/v1/*
WS   /ws
WS   /ws/terminal
WS   /ws/apps/deploy
WS   /ws/system/logs
WS   /ws/vm/*
```

For `nastty` first version, keep:

```text
GET  /health
POST /api/login
POST /api/logout
GET  /api/auth/check
GET  /api/openapi.json, optional
ANY  /api/v1/*
WS   /ws
```

Defer:

```text
/ws/terminal
/ws/apps/deploy
/ws/vm/*
/api/public/share/*
/api/files/*, unless you want file browser API early
```

## JSON-RPC dispatcher

The core dispatcher is:

```text
engine/nasty-engine/src/router/mod.rs
```

Flow:

```text
raw WebSocket text
  -> serde_json parse into Request
  -> role / permission check
  -> route by method prefix
  -> service call
  -> audit mutation
  -> broadcast event if mutation changed state
  -> serialize Response
```

Domain routing is by first method segment:

```text
auth.*          -> router/auth.rs
audit.*         -> router/audit.rs
alert.*         -> router/alerts.rs
notifications.* -> router/notifications.rs
backup.*        -> router/backup.rs
fs.*            -> router/fs.rs
device.*        -> router/fs.rs
bcachefs.*      -> router/bcachefs.rs
subvolume.*     -> router/subvolume.rs
snapshot.*      -> router/snapshot.rs
share.*         -> router/share.rs
guestshare.*    -> router/guestshare.rs
smb.*           -> router/smb.rs
domain.*        -> router/domain.rs
service.*       -> router/service.rs
system.*        -> router/system.rs
firmware.*      -> router/system.rs
vm.*            -> router/vm.rs
apps.*          -> router/apps.rs
```

For `nastty`, this dispatcher pattern is good. Keep it, but make modules
pluggable.

## REST gateway

There is also a REST wrapper:

```text
engine/nasty-engine/src/rest_gateway.rs
```

It maps:

```text
/api/v1/system/info
```

to:

```text
system.info
```

And:

```text
/api/v1/fs/create
```

to:

```text
fs.create
```

It still calls the same JSON-RPC dispatcher internally, so auth, roles, audit,
events, and slow-call logging behave the same.

This is useful for `nastty` because a CLI can use plain HTTP first, and a richer
UI can use WebSocket later.

## Event model

After a successful mutation, the server broadcasts a notification over the same
WebSocket connection:

```json
{
  "jsonrpc": "2.0",
  "method": "event",
  "params": {
    "collection": "filesystem"
  }
}
```

Collections include things like:

```text
filesystem
subvolume
snapshot
share.nfs
share.smb
share.iscsi
share.nvmeof
protocol
settings
tuning
nut
tailscale
alert
```

This means a future GUI/TUI does not need to poll everything constantly. It can
listen for events and refresh only affected collections.

## API capability groups

Upstream registry currently exposes about 297 methods.

Useful capability groups:

```text
auth.*          login, logout, users, API tokens, roles
system.*        info, health, settings, network, logs, updates, tuning
device.*        list disks, wipe disks, set disk type metadata
fs.*            bcachefs filesystems
subvolume.*     filesystem/block subvolumes
snapshot.*      snapshots and rollback
share.nfs.*     NFS shares
share.smb.*     SMB shares
share.iscsi.*   iSCSI targets, LUNs, ACLs, portals
share.nvmeof.*  NVMe-oF subsystems, namespaces, ports, hosts
smb.*           SMB users and groups
backup.*        backup profiles, jobs, repo init/check
vm.*            QEMU/KVM
apps.*          Docker/Compose
notifications.* alert delivery config
firmware.*      firmware checks and updates
```

Recommended first `nastty` capability set:

```text
auth.*
system.info
system.health
device.list
device.wipe, maybe later
fs.list
fs.get
fs.create
fs.mount
fs.unmount
fs.destroy, dangerous; add later or guard heavily
subvolume.*
snapshot.*
share.nfs.*
share.smb.*
smb.user.*
smb.group.*
service.protocol.*
```

Defer:

```text
apps.*
vm.*
backup.*
firmware.*
notifications.*
OIDC/WebAuthn
guestshare.*
terminal
Tailscale
Secure Boot
```

## Storage model

Storage is built around bcachefs.

Important paths:

```text
/fs                         filesystem mount base
/var/lib/nasty/fs-state.json
/var/lib/nasty/mount-state.json
/var/lib/nasty/keys
/var/lib/nasty/shares/nfs
/var/lib/nasty/shares/smb
/var/lib/nasty/shares/iscsi
/var/lib/nasty/shares/nvmeof
/var/lib/nasty/protocols.json
```

Filesystem create flow:

```text
validate devices
validate options
run bcachefs format
create /fs/<name>
unlock if encrypted
run bcachefs mount
apply I/O scheduler
save mount state
return filesystem info
```

Mount restore flow:

```text
load /var/lib/nasty/fs-state.json
wait for udev
wait for expected devices
mount each known filesystem under /fs/<name>
record mount failures
```

## Protocol/service model

Protocol state lives at:

```text
/var/lib/nasty/protocols.json
```

Supported protocol toggles:

```text
nfs
smb
iscsi
nvmeof
nut
ssh
avahi
smart
rest-server
```

Enable flow:

```text
set protocol enabled in JSON
prepare config
modprobe needed kernel modules
systemctl start needed services
rollback JSON state if service start fails
return status
```

Service mapping examples:

```text
nfs      -> nfs-server.service
smb      -> samba-smbd.service, samba-nmbd.service, samba-wsdd.service
iscsi    -> target.service
nvmeof   -> no daemon; configfs based
ssh      -> sshd.service
avahi    -> avahi-daemon.service
smart    -> smartd.service
```

For a local-only modular server, this is still relevant. The API server will
need permission to call `systemctl`, `modprobe`, `bcachefs`, `exportfs`,
`smbcontrol`, etc., unless those are moved behind adapters.

## NFS behavior

NFS shares are stored in:

```text
/var/lib/nasty/shares/nfs
```

Generated exports are written to:

```text
/etc/exports.d/nasty-<id>.exports
```

Then the engine runs:

```text
exportfs -ra
```

NFS share create checks:

- path exists
- path canonicalizes under `/fs/`
- client host/options are sanitized
- stable `fsid` is added if missing

## SMB behavior

SMB shares are stored in:

```text
/var/lib/nasty/shares/smb
```

Generated per-share configs:

```text
/etc/samba/nasty.d/<id>.conf
```

Main generated include file:

```text
/etc/samba/smb.nasty.conf
```

SMB reload command:

```text
smbcontrol all reload-config
```

SMB share create checks:

- share name valid
- path exists
- path canonicalizes under `/fs/`
- valid users are sanitized
- Time Machine shares must be authenticated and writable

## Metrics

> **Local nastty note:** the upstream architecture below used a separate
> process when this scan was written. This repository now embeds collection,
> history, RPC responses, and Prometheus output directly in `nasttyd` on port
> 2137. Running a second metrics daemon is neither required nor supported by
> the local two-binary design.

The metrics daemon is separate:

```text
engine/nasty-metrics/src/main.rs
```

It binds to:

```text
0.0.0.0:2138
```

Routes:

```text
GET /metrics
GET /api/stats
GET /api/disks
GET /api/kernel_errors
GET /api/history
GET /health
```

It collects:

- system stats every 5s
- disk health every 60s
- bcachefs metrics every 5s
- kernel errors every 30s

For `nastty`, this can be optional or merged later.

## What should `nastty` copy conceptually

Good patterns to keep:

1. Local-only backend on loopback.
2. JSON-RPC method names grouped by domain.
3. REST gateway as a thin wrapper over the same dispatcher.
4. Central auth/role check before dispatch.
5. Mutation audit log.
6. Event bus for changed collections.
7. Persistent JSON state under one state directory.
8. Explicit startup restore phases.
9. Service modules for storage/sharing/system.

Things to change:

1. Do not keep one huge `AppState` forever.
2. Do not require WebUI/static files for API mode.
3. Make modules optional at compile time or config time.
4. Make command execution injectable/testable instead of hard-coded shellouts everywhere.
5. Replace default `admin/admin` with explicit bootstrap.
6. Keep local-only as the first supported mode.

## Suggested `nastty` crate layout

```text
crates/nastty-api
  Axum server, route registration, WebSocket, REST gateway

crates/nastty-rpc
  JSON-RPC Request/Response, method registry, dispatcher traits

crates/nastty-auth
  users, sessions, API tokens, roles

crates/nastty-core
  config, state paths, errors, command runner trait

crates/nastty-storage
  bcachefs filesystem, device, subvolume, snapshot logic

crates/nastty-sharing
  NFS and SMB first; iSCSI/NVMe-oF later

crates/nastty-system
  protocol lifecycle, health, basic system info

crates/nastty-cli
  local CLI over HTTP/WebSocket API
```

First runnable binary:

```text
nasttyd
```

with:

```text
nasttyd serve --listen 127.0.0.1:2137 --state-dir /var/lib/nastty
```

For development:

```text
nasttyd serve --listen 127.0.0.1:2137 --state-dir ./var
```

## Minimal first milestone

Milestone 1 should prove the server shape, not the whole NAS:

1. Start local API server.
2. `GET /health`.
3. `POST /api/login`.
4. WebSocket `/ws`.
5. JSON-RPC call:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "system.info"
}
```

6. REST equivalent:

```text
GET /api/v1/system/info
```

7. One event broadcast after a fake mutation.

Milestone 2:

```text
device.list
fs.list
service.protocol.list
```

Milestone 3:

```text
fs.create
fs.mount
fs.unmount
share.nfs.create
share.smb.create
```

## Main conclusion

The GUI is not the core product. The API server is.

For `nastty`, start by building a clean local API daemon and CLI around the
upstream engine concepts:

```text
local daemon
JSON-RPC over WebSocket
REST wrapper
modular service crates
bcachefs/NFS/SMB first
everything else later
```

That gives a NAS server without inheriting the whole appliance/UI stack.
