#!/usr/bin/env bash
# Runs blossom-compliance against the built binary (or, with BLOSSOM_IMAGE set, against that
# container image), through the test-only proxy, and fails on
# its results.json. Needs: cargo (not with BLOSSOM_IMAGE; docker instead), node, bun, git, curl.
# Run from anywhere.
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
  if [[ -n ${BLOSSOM_IMAGE:-} ]]; then docker rm -f blossom-compliance >/dev/null 2>&1 || true; fi
  rm -rf "$work"
}
trap cleanup EXIT

write_port=${WRITE_PORT:-3101} public_port=${PUBLIC_PORT:-3100} proxy_port=${PROXY_PORT:-8080}

if [[ -z ${BLOSSOM_IMAGE:-} ]]; then
  cargo build --locked --manifest-path "$root/Cargo.toml"
  binary=${BLOSSOM_BIN:-$root/target/debug/blossom}
fi

git init -q "$work/suite"
git -C "$work/suite" fetch -q --depth 1 https://github.com/v0l/blossom-compliance "$COMPLIANCE_REF"
git -C "$work/suite" checkout -q FETCH_HEAD
(cd "$work/suite" && bun install --frozen-lockfile >/dev/null)

if [[ -n ${BLOSSOM_IMAGE:-} ]]; then
  docker run -d --name blossom-compliance \
    -p "127.0.0.1:$write_port:3001" -p "127.0.0.1:$public_port:3000" \
    -e TOON_BLOB_PUBLIC_URL="http://127.0.0.1:$proxy_port" "$BLOSSOM_IMAGE" >/dev/null
  # Follow the container's log so the failure dump below has it.
  docker logs -f blossom-compliance >"$work/app.log" 2>&1 &
  pids+=($!)
else
  "$binary" --write-addr "127.0.0.1:$write_port" --public-addr "127.0.0.1:$public_port" \
    --public-url "http://127.0.0.1:$proxy_port" --storage-dir "$work/data" >"$work/app.log" 2>&1 &
  pids+=($!)
fi
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
