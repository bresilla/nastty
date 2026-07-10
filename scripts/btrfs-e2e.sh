#!/usr/bin/env bash
# End-to-end btrfs test against a real (loop-device) filesystem.
# Needs root: mkfs, mount, and /var/lib/nasty all require it.
#
#   sudo ./scripts/btrfs-e2e.sh [path/to/nasttyd]
#
# Uses its own state dir? No — paths are fixed upstream (/var/lib/nasty,
# /fs), so this script creates them if missing and cleans up only what it
# made (loop devices, the test filesystem, its own state entries).

set -euo pipefail

BIN="${1:-./target/debug/nasttyd}"
PORT=2199
BASE="http://127.0.0.1:$PORT"
FS_NAME="e2etank"
WORK="$(mktemp -d /tmp/nastty-btrfs-e2e.XXXXXX)"
LOOPS=()
SERVER_PID=""

say()  { printf '\033[1;36m== %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31mFAIL: %s\033[0m\n' "$*"; exit 1; }

cleanup() {
  set +e
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
  umount "/fs/$FS_NAME" 2>/dev/null
  for l in "${LOOPS[@]:-}"; do losetup -d "$l" 2>/dev/null; done
  rm -rf "$WORK"
  # Drop only our test entry from the btrfs state file.
  if [ -f /var/lib/nasty/btrfs-state.json ]; then
    python3 - <<PY 2>/dev/null
import json
p = "/var/lib/nasty/btrfs-state.json"
s = json.load(open(p))
if s.get("filesystems", {}).pop("$FS_NAME", None) is not None:
    json.dump(s, open(p, "w"), indent=2)
PY
  fi
  rmdir "/fs/$FS_NAME" 2>/dev/null
}
trap cleanup EXIT

[ "$(id -u)" = 0 ] || fail "run with sudo (mkfs/mount need root)"
[ -x "$BIN" ] || fail "nasttyd binary not found at $BIN (run: cargo build --bin nasttyd)"
command -v mkfs.btrfs >/dev/null || fail "btrfs-progs not installed"

say "setting up state dirs and two 1 GiB loop devices"
mkdir -p /var/lib/nasty /fs
for i in 1 2; do
  truncate -s 1G "$WORK/disk$i.img"
  LOOPS+=("$(losetup --find --show "$WORK/disk$i.img")")
done
echo "   loops: ${LOOPS[*]}"

say "starting nasttyd on :$PORT"
"$BIN" --listen "127.0.0.1:$PORT" >"$WORK/nasttyd.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 20); do
  curl -sf --max-time 1 "$BASE/health" >/dev/null && break
  sleep 0.5
done
curl -sf "$BASE/health" >/dev/null || { cat "$WORK/nasttyd.log"; fail "server did not come up"; }

api() { # api METHOD PATH [JSON]
  local method=$1 path=$2 body=${3:-}
  if [ -n "$body" ]; then
    curl -sf -X "$method" -H "Authorization: Bearer $TOK" \
      -H 'content-type: application/json' -d "$body" "$BASE$path"
  else
    curl -sf -X "$method" -H "Authorization: Bearer $TOK" "$BASE$path"
  fi
}

say "login + password change"
TOK=$(curl -sf -X POST "$BASE/api/login" -H 'content-type: application/json' \
  -d '{"username":"admin","password":"admin"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])' ) \
  || { # not a fresh install: state dir persists a changed password
       TOK=$(curl -sf -X POST "$BASE/api/login" -H 'content-type: application/json' \
         -d '{"username":"admin","password":"e2e-password-1"}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])') \
         || fail "login failed with both default and e2e passwords"; }
api POST /api/v1/auth/change_password '{"old_password":"admin","new_password":"e2e-password-1"}' >/dev/null 2>&1 || true

say "fs.create (btrfs raid1 on ${LOOPS[0]} + ${LOOPS[1]})"
api POST /api/v1/fs/create "{\"name\":\"$FS_NAME\",\"devices\":[\"${LOOPS[0]}\",\"${LOOPS[1]}\"],\"raid\":\"raid1\",\"compression\":\"zstd\"}" \
  | python3 -m json.tool | sed -n '1,12p'
mountpoint -q "/fs/$FS_NAME" || fail "filesystem not mounted at /fs/$FS_NAME"

say "fs.list shows it with backend=btrfs"
api GET /api/v1/fs/list | grep -o "\"name\":\"$FS_NAME\"" >/dev/null || fail "fs.list missing $FS_NAME"

say "subvolume create + list"
api POST /api/v1/subvolume/create "{\"filesystem\":\"$FS_NAME\",\"name\":\"data\"}" >/dev/null
api GET "/api/v1/subvolume/list?filesystem=$FS_NAME" | grep -o '"path":"data"' >/dev/null || fail "subvolume missing"

say "snapshot create + list + clone"
api POST /api/v1/snapshot/create "{\"filesystem\":\"$FS_NAME\",\"subvolume\":\"data\",\"name\":\"first\"}" >/dev/null
api GET "/api/v1/snapshot/list?filesystem=$FS_NAME" | grep -o '"name":"data@first"' >/dev/null || fail "snapshot missing"
api POST /api/v1/snapshot/clone "{\"filesystem\":\"$FS_NAME\",\"subvolume\":\"data\",\"snapshot\":\"first\",\"new_name\":\"restored\"}" >/dev/null
api GET "/api/v1/subvolume/list?filesystem=$FS_NAME" | grep -o '"path":"restored"' >/dev/null || fail "clone missing"

say "fs.usage + scrub"
api GET "/api/v1/fs/usage?name=$FS_NAME" | python3 -m json.tool
api POST /api/v1/fs/scrub/start "{\"name\":\"$FS_NAME\"}" >/dev/null
sleep 1
api GET "/api/v1/fs/scrub/status?name=$FS_NAME" | python3 -m json.tool

say "bcachefs-only method returns a clean error"
out=$(curl -s -X POST -H "Authorization: Bearer $TOK" -H 'content-type: application/json' \
  -d "{\"name\":\"$FS_NAME\",\"passphrase\":\"x\"}" "$BASE/api/v1/fs/unlock")
echo "   $out"
echo "$out" | grep -q "not supported on btrfs" || fail "expected clean not-supported error"

say "unmount + destroy"
api POST /api/v1/fs/unmount "{\"name\":\"$FS_NAME\"}" >/dev/null
api POST /api/v1/fs/destroy "{\"name\":\"$FS_NAME\"}" >/dev/null
api GET /api/v1/fs/list | grep -o "\"name\":\"$FS_NAME\"" && fail "filesystem still listed after destroy"

say "ALL OK — btrfs backend works end-to-end"
