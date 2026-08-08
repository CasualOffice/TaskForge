#!/usr/bin/env bash
#
# The developer profile, actually started (F-010).
#
# `docker compose config` proves the file parses. It does not prove the profile
# comes up, and it does not check the invariant the file states in its own
# header:
#
#   > Opt-in only: `docker compose --profile full up -d`.
#   > Nothing in the default profile may depend on these.
#
# That sentence is the whole point of the profile. docs/48 §Profile 1 makes the
# single-node deployment — PostgreSQL and nothing else — a supported target, and
# the way that stays true is that a developer's default `docker compose up -d`
# never quietly starts a mail catcher and an object store. A dependency added to
# the default profile would be invisible to a syntax check and obvious here.
#
#   ./scripts/verify-dev-profile.sh
#
set -euo pipefail
cd "$(dirname "$0")/.."

PROJECT=tf-dev-profile-verify
DC="docker compose -p $PROJECT -f docker-compose.yml"

step() { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
red()  { printf '\033[31m%s\033[0m\n' "$*"; }

# --profile full is load-bearing in the TEARDOWN, not just the start: this
# script starts the profiled services itself, and `down` without the flag left
# minio running after a failing run. -v removes the named volumes too, without
# which every run leaves pgdata behind.
cleanup() {
  docker compose -p "$PROJECT" --profile full -f docker-compose.yml \
    down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup

step "The default profile starts"
$DC up -d >/dev/null

# The healthcheck is the file's own definition of ready, so use it rather than a
# second opinion. -h 127.0.0.1 for the reason in verify-schema.sh: during initdb
# the entrypoint runs a temporary server on the unix socket only.
for _ in $(seq 1 60); do
  $DC exec -T postgres pg_isready -h 127.0.0.1 -U taskforge -q 2>/dev/null && break
  sleep 1
done
if ! $DC exec -T postgres pg_isready -h 127.0.0.1 -U taskforge -q 2>/dev/null; then
  red "  ❌ PostgreSQL never became ready"; exit 1
fi
echo "  ✅ PostgreSQL is reachable"

step "It is PostgreSQL and nothing else"
# The invariant from the file header. `--profile full` services must not start
# unless asked for.
running=$($DC ps --services --status running | sort | tr '\n' ' ')
expected="postgres "
if [ "$running" != "$expected" ]; then
  red "  ❌ default profile started: ${running:-nothing}"
  red "     expected exactly: $expected"
  red "     docs/48 §Profile 1 — the single-node profile is a supported target,"
  red "     and it stops being one the moment the dev default needs more."
  exit 1
fi
echo "  ✅ only postgres is running; mailpit and minio stayed opt-in"

step "The full profile still works when asked for"
docker compose -p $PROJECT --profile full -f docker-compose.yml up -d >/dev/null
full=$(docker compose -p $PROJECT --profile full -f docker-compose.yml ps --services --status running | sort | tr '\n' ' ')
for svc in mailpit minio postgres; do
  case "$full" in
    *"$svc"*) ;;
    *) red "  ❌ --profile full did not start $svc (got: $full)"; exit 1 ;;
  esac
done
echo "  ✅ mailpit and minio start on --profile full"

printf '\n\033[32mDeveloper profile verified.\033[0m\n'
