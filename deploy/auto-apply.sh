#!/usr/bin/env bash
#
# Apply what was merged. Run by systemd on a timer (toon-auto-apply-blossom.timer), from the
# checkout at /root/blossom on the devnet host. The same loop as the other nodes there
# (echo-merchant's deploy/auto-apply.sh, connector ADR 0068): pull-based, so nothing outside the
# box can make it deploy.
#
# It refuses rather than guesses:
#   * a dirty working tree means a human is mid-operation here: stop, loudly;
#   * only a fast-forward is applied, never a merge or a reset;
#   * after `up -d` the blob app must be running and accepting connections on its public port,
#     or this exits non-zero so `systemctl status` and the journal show it;
#   * a failed apply is retried on every run until it succeeds (deploy/.applied names the last
#     commit that did).
#
# On the devnet host this bundle runs behind the shared edge, with its own connector and Caddy
# off: deploy/.env sets COMPOSE_FILE=docker-compose.yml:docker-compose.shared-edge.yml. The blob
# app mounts no config file, so `up -d` sees every change (image or .env) by itself and there is
# no connector here to restart.
set -euo pipefail

REPO_DIR=$(cd "$(dirname "$0")/.." && pwd)
DEPLOY_DIR="$REPO_DIR/deploy"
cd "$REPO_DIR"

# One apply at a time, and never one racing a human. Named per node, so it never serialises
# against another node's apply on the same host. Overridable only for tests.
LOCK_FILE=${TOON_AUTOAPPLY_LOCK:-/var/lock/toon-auto-apply-blossom.lock}
exec 9>"$LOCK_FILE"
flock -n 9 || { echo "another apply is already running; leaving it alone"; exit 0; }

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "REFUSING: the working tree at $REPO_DIR is dirty."
  echo "Someone is editing on the box. Commit, stash or discard it, then this resumes on its own."
  exit 1
fi

git fetch -q origin main
LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse origin/main)

# The commit the last run applied and verified. Missing reads as "needs an apply": re-applying a
# healthy box is a no-op, where guessing the other way could hide a first apply that failed.
APPLIED_FILE="$DEPLOY_DIR/.applied"
APPLIED=$(cat "$APPLIED_FILE" 2>/dev/null || true)

if [ "$LOCAL" = "$REMOTE" ] && [ "$LOCAL" = "$APPLIED" ]; then
  exit 0
elif [ "$LOCAL" != "$REMOTE" ]; then
  echo "applying ${LOCAL:0:7} -> ${REMOTE:0:7}"
  git merge --ff-only origin/main
else
  echo "retrying ${LOCAL:0:7}: the last apply did not finish (deploy/.applied is '${APPLIED:-<none>}')"
fi

cd "$DEPLOY_DIR"
[ -f .env ] || { echo "FAILED: $DEPLOY_DIR/.env is missing (DOMAIN, STORAGE_CAP_BYTES, COMPOSE_FILE)." >&2; exit 1; }

# docker compose reads COMPOSE_FILE out of .env itself, so no `-f` is passed when it is set: an
# explicit `-f` would override it and run the base file alone, connector and Caddy included.
set -a; . ./.env; set +a
if [ -n "${COMPOSE_FILE:-}" ]; then
  COMPOSE=()
else
  COMPOSE=(-f docker-compose.yml)
fi

if ! docker compose "${COMPOSE[@]}" pull; then
  echo "FAILED: 'docker compose pull' could not get the images for ${REMOTE:0:7} (its message is above)." >&2
  exit 1
fi
if ! docker compose "${COMPOSE[@]}" up -d; then
  echo "FAILED: 'docker compose up -d' failed for ${REMOTE:0:7} (its message is above)." >&2
  exit 1
fi

# The image has no HTTP client and no health route, so this asks the container itself whether its
# public port accepts a connection (bash's /dev/tcp). The app binds its ports just before it opens
# its storage and exits if that fails, so one accepted connection could be a dying app: it must
# accept twice, 5 seconds apart.
listening() {
  docker compose "${COMPOSE[@]}" exec -T blossom bash -c '</dev/tcp/127.0.0.1/3000' 2>/dev/null
}
wait_blossom_listening() {
  for _ in $(seq 1 40); do
    if listening && sleep 5 && listening; then
      return 0
    fi
    sleep 3
  done
  echo "FAILED: the blob app is not accepting connections on :3000 after applying ${REMOTE:0:7}."
  docker compose "${COMPOSE[@]}" logs --tail 40 blossom || true
  return 1
}

wait_blossom_listening || exit 1

# Written only now, after the pull, `up -d` and the check have all succeeded. Gitignored.
printf '%s\n' "$REMOTE" > "$APPLIED_FILE"

echo "applied ${REMOTE:0:7}; the blob app is up."
