#!/usr/bin/env bash
# Runs blossom-compliance against the built binary, through the test-only proxy, and fails on
# its results.json. Needs: cargo, node, bun, git, curl. Run from anywhere.
#
# The suite is fetched, not vendored: its repository has no LICENSE file. It is pinned to a
# commit so a change upstream never turns CI red by itself; bump COMPLIANCE_REF deliberately.
set -euo pipefail

COMPLIANCE_REF=${COMPLIANCE_REF:-47c9969c25aa95b139aa1395b729e1335e9c32c1}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
work=$(mktemp -d)
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  rm -rf "$work"
}
trap cleanup EXIT

write_port=${WRITE_PORT:-3101} public_port=${PUBLIC_PORT:-3100} proxy_port=${PROXY_PORT:-8080}

cargo build --locked --manifest-path "$root/Cargo.toml"
binary=${BLOSSOM_BIN:-$root/target/debug/blossom}

git init -q "$work/suite"
git -C "$work/suite" fetch -q --depth 1 https://github.com/v0l/blossom-compliance "$COMPLIANCE_REF"
git -C "$work/suite" checkout -q FETCH_HEAD
(cd "$work/suite" && bun install --frozen-lockfile >/dev/null)

"$binary" --write-addr "127.0.0.1:$write_port" --public-addr "127.0.0.1:$public_port" \
  --public-url "http://127.0.0.1:$proxy_port" --storage-dir "$work/data" \
  --storage-cap-bytes 1000000000 >"$work/app.log" 2>&1 &
pids+=($!)
node "$here/proxy.mjs" "$proxy_port" "http://127.0.0.1:$write_port" "http://127.0.0.1:$public_port" \
  >"$work/proxy.log" 2>&1 &
pids+=($!)

# Ready once both ports answer through the proxy: it answers 502 while the app is still down.
reaches_app() {
  local read write
  read=$(curl -s --max-time 5 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$proxy_port/0")
  write=$(curl -s --max-time 5 -o /dev/null -w '%{http_code}' -X PUT "http://127.0.0.1:$proxy_port/upload")
  [[ $read != 000 && $read != 502 && $write != 000 && $write != 502 ]]
}
for _ in $(seq 30); do
  reaches_app && break
  sleep 1
done
reaches_app || {
  echo "the proxy never reached the app" >&2
  cat "$work/app.log" "$work/proxy.log" >&2
  exit 1
}

(cd "$work/suite" && BLOSSOM_SERVER_URL="http://127.0.0.1:$proxy_port" bun run report.ts)
node "$here/check.mjs" "$work/suite/report/results.json" "$here/expected-failures.json"
