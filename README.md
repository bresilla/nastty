# nastty

`nastty` is a Linux NAS server and terminal interface for bcachefs.

It builds one executable with two commands:

```sh
nastty serve   # API server, storage control, protocols, and metrics
nastty tui     # interactive terminal client
```

## Install

Download the binary for your architecture from the
[GitHub Releases](https://github.com/bresilla/nastty/releases) page.

```sh
# amd64 example
tar -xzf nastty-linux-amd64.tar.gz
sudo install -m 0755 nastty-linux-amd64 /usr/local/bin/nastty
nastty --version
```

Use `arm64` instead of `amd64` on a 64-bit ARM system.

## Requirements

- Linux
- [bcachefs-tools](https://github.com/koverstreet/bcachefs-tools)
- root access for system changes such as formatting, mounting, and services
- NFS or Samba packages when using those sharing protocols

On Ubuntu:

```sh
sudo apt install nfs-kernel-server samba
```

## Run

Start the server:

```sh
sudo nastty serve
```

Then open the TUI in another terminal:

```sh
nastty tui
```

The default address is `http://127.0.0.1:2137`. The first login is
`admin` / `admin`; nastty immediately asks you to change the password.

Useful options:

```sh
nastty serve --listen 0.0.0.0:2137
nastty serve --allow-missing-deps
nastty tui --server http://192.168.1.10:2137 --user admin
```

## TUI controls

| Key | Action |
| --- | --- |
| `←` / `→`, `h` / `l` | Change section |
| `Tab` / `Shift-Tab` | Change view |
| `↑` / `↓`, `j` / `k` | Select an item |
| `Enter` | Open controls for the selected item |
| `Space` | Open details |
| `/` or `:` | Open the command palette |
| `?` | Show help for the current view |
| `1`–`9`, `0` | Jump directly to a view |

The TUI manages devices, filesystems, subvolumes, snapshots, files, shares,
protocols, users, alerts, and system settings.

## API

```sh
curl http://127.0.0.1:2137/health
curl http://127.0.0.1:2137/metrics
```

The TUI connects to the JSON-RPC WebSocket endpoint at `/ws`.

## Development

```sh
make build          # debug build
make build-release  # optimized build
make test           # tests
make verify         # complete validation
make serve          # run the server
make tui            # run the TUI
```

## Releases

Publishing a GitHub Release builds native Linux `amd64` and `arm64` binaries
and attaches each binary, a `.tar.gz` archive, and SHA-256 checksums.
