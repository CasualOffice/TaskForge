#!/usr/bin/env bash
# Empty machine to a login screen, in one command.
#
#   scripts/dev-up.sh          start everything and print how to log in
#   scripts/dev-up.sh --reset  destroy the database first, then start
#   scripts/dev-up.sh --down   stop everything and remove the container
#
# # Why this exists
#
# Every gate in this repository proves the product is *correct*. None of them
# proves it is *runnable*: the test suites build their own database, seed their
# own rows and drive the router in-process, so all of them pass on a tree where
# nobody can actually log in. This script is the only thing that runs the real
# binary, against a real database, with a real account, and it exists because
# "the tests pass" and "you can open it" turned out to be very different claims.
#
# It is deliberately NOT deploy/docker-compose.yml. That file is Profile 1 from
# docs/48 — a published image, restart policies, an .env you must edit — and it
# is what an operator uses. This is what a developer uses, from source, with
# defaults that would be indefensible in production and are stated as such
# below.

set -euo pipefail
cd "$(dirname "$0")/.."

CONTAINER=taskforge-dev
PGPORT=${TF_DEV_PGPORT:-55433}
API_PORT=${TF_DEV_API_PORT:-8080}
WEB_PORT=${TF_DEV_WEB_PORT:-5173}

# Development credentials, hard-coded on purpose. A script that generated them
# would have to store them somewhere, and a developer who cannot predict the
# password cannot log in after closing the terminal. Nothing here is reachable
# from outside the loopback interface, and scripts/verify-no-secrets.sh knows
# this file by name.
DEMO_EMAIL=${TF_DEV_EMAIL:-demo@taskforge.test}
DEMO_PASSWORD=${TF_DEV_PASSWORD:-taskforge demo password}
DB_PASSWORD=devpassword

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
step()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
note()  { printf '  %s\n' "$*"; }

psql_owner() { docker exec -i "$CONTAINER" psql -U tf -d tf "$@"; }

down() {
  step "Stopping"
  # `nohup`, not `setsid`: macOS has no setsid. Children are killed by pid, with
  # a sweep for grandchildren — pnpm dev execs vite as a child of a child.
  for pidfile in .dev/api.pid .dev/web.pid; do
    if [[ -f "$pidfile" ]]; then
      pkill -P "$(cat "$pidfile")" 2>/dev/null || true; kill "$(cat "$pidfile")" 2>/dev/null || true
      rm -f "$pidfile"
    fi
  done
  docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true
  green "stopped"
}

RESET=0
case "${1:-}" in
  --down)  down; exit 0 ;;
  --reset) RESET=1 ;;
  "")      ;;
  *)       red "unknown argument: $1"; exit 2 ;;
esac

mkdir -p .dev

# Checked first, and by name. The daemon on this machine has died mid-session
# more than once, and every downstream symptom — a container that will not
# start, a gate reporting "sequential scan", a suite failing on connection
# refused — reads as a product bug rather than as a stopped daemon.
if ! docker info >/dev/null 2>&1; then
  red "the Docker daemon is not running."
  note "macOS:  open -a Docker    (then wait ~30 s)"
  note "Linux:  sudo systemctl start docker"
  exit 1
fi

# ── Database ────────────────────────────────────────────────────────────────
if [[ $RESET -eq 1 ]]; then
  step "Resetting the database"
  docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true
fi

if docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
  step "PostgreSQL is already running"
else
  step "Starting PostgreSQL 16"
  docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true
  # -v on removal matters: the image declares a VOLUME, so every run without it
  # orphans ~600 MB. Repeated enough times that has filled this machine's disk.
  docker run -d --name "$CONTAINER" \
    -e POSTGRES_USER=tf -e POSTGRES_PASSWORD=tf -e POSTGRES_DB=tf \
    -p "127.0.0.1:${PGPORT}:5432" postgres:16-alpine >/dev/null
  # -h 127.0.0.1 is load-bearing: during initdb the entrypoint runs a temporary
  # server on the unix socket only, so a socket-based pg_isready reports ready
  # seconds before the real server exists.
  for _ in $(seq 1 60); do
    docker exec "$CONTAINER" pg_isready -h 127.0.0.1 -U tf -q 2>/dev/null && break
    sleep 1
  done
  docker exec "$CONTAINER" pg_isready -h 127.0.0.1 -U tf -q 2>/dev/null || {
    red "PostgreSQL did not become ready"; exit 1; }
fi

# Migrations are not idempotent — 0001 creates types with no IF NOT EXISTS — so
# re-running them against a database that already has them fails on the second
# statement and looks like a broken migration. A dev database that survives a
# restart is the whole point of keeping the container, so detect and skip.
ALREADY=$(psql_owner -tAq -c "SELECT to_regclass('public.task') IS NOT NULL" 2>/dev/null || echo f)
if [[ "$ALREADY" == "t" ]]; then
  step "Schema is already present — skipping migrations"
  note "run scripts/dev-up.sh --reset to rebuild it from scratch"
else
step "Applying migrations"
for f in migrations/*.sql; do
  if psql_owner -v ON_ERROR_STOP=1 -q < "$f" >/dev/null 2>&1; then
    printf '  ✅ %s\n' "$(basename "$f")"
  else
    red "  ❌ $(basename "$f")"
    psql_owner -v ON_ERROR_STOP=1 -q < "$f" 2>&1 | head -20
    exit 1
  fi
done
fi

# The API refuses to start as a superuser (see crates/casual-task-api/src/main.rs
# and migration 0012), and it is right to: a superuser bypasses every RLS policy,
# so tenant isolation would be silently inert in the very environment where a
# developer decides the product works. Connecting as taskforge_app here is not
# ceremony; it is what makes the dev stack behave like production.
psql_owner -q -c "ALTER ROLE taskforge_app WITH LOGIN PASSWORD '${DB_PASSWORD}';" >/dev/null
psql_owner -q -c "ALTER ROLE taskforge_dispatcher WITH LOGIN PASSWORD '${DB_PASSWORD}';" >/dev/null

APP_DSN="postgres://taskforge_app:${DB_PASSWORD}@127.0.0.1:${PGPORT}/tf"
# A second DSN, as taskforge_dispatcher (D-060). The dispatcher bypasses RLS
# because a background worker cannot know every workspace id; giving that
# capability to taskforge_app instead would hand it to every request the
# product serves.
DISPATCHER_DSN="postgres://taskforge_dispatcher:${DB_PASSWORD}@127.0.0.1:${PGPORT}/tf"

# ── The demo account ────────────────────────────────────────────────────────
step "Creating the demo account"
# Hashed by casual-task-identity, not by a generic tool: the Argon2 parameters
# are a security decision docs/40 fixes (64 MiB, t=3, p=4), and a demo login
# protected by different parameters is a demo of a different system.
HASH=$(cargo run --quiet --release -p casual-task-seed -- --hash-password "$DEMO_PASSWORD")
DEMO_USER_ID=$(psql_owner -tAq <<SQL
INSERT INTO user_account (id, email, display_name)
VALUES (gen_random_uuid(), '${DEMO_EMAIL}', 'Demo User')
ON CONFLICT (email) DO UPDATE SET display_name = EXCLUDED.display_name
RETURNING id;
SQL
)
psql_owner -q <<SQL
INSERT INTO user_credential (user_id, password_hash)
VALUES ('${DEMO_USER_ID}', '${HASH}')
ON CONFLICT (user_id) DO UPDATE SET password_hash = EXCLUDED.password_hash;
SQL
note "$DEMO_EMAIL"

# ── The API ─────────────────────────────────────────────────────────────────
step "Building the API"
cargo build --release -p casual-task-api

step "Starting the API on :${API_PORT}"
if [[ -f .dev/api.pid ]]; then pkill -P "$(cat .dev/api.pid)" 2>/dev/null || true; kill "$(cat .dev/api.pid)" 2>/dev/null || true; fi
# TF_SECRET_KEY is fixed so sessions survive a restart of this script — a
# developer logged out by every rebuild stops using the dev stack.
# Every variable Config::from_env demands. Listed in full rather than relying on
# defaults: the binary refuses to start on a missing one, by design (docs/48,
# "a misconfigured deployment must not start"), and the whole value of that
# refusal is lost if this script has to be debugged one variable at a time.
env DATABASE_URL="$APP_DSN" \
    DISPATCHER_DATABASE_URL="$DISPATCHER_DSN" \
    TF_BIND_ADDR="127.0.0.1:${API_PORT}" \
    TF_SECRET_KEY="dev-only-secret-key-not-for-any-real-deployment" \
    TF_PUBLIC_URL="http://127.0.0.1:${WEB_PORT}" \
    TF_ATTACHMENT_ORIGIN="http://127.0.0.1:${API_PORT}" \
    TF_STORAGE_BACKEND="fs" \
    TF_STORAGE_PATH="$(pwd)/.dev/objects" \
    TF_WORKER_EMBEDDED="true" \
  nohup ./target/release/casual-task-api > .dev/api.log 2>&1 &
echo $! > .dev/api.pid

for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:${API_PORT}/health/ready" >/dev/null 2>&1; then break; fi
  sleep 1
done
if ! curl -fsS "http://127.0.0.1:${API_PORT}/health/ready" >/dev/null 2>&1; then
  red "the API did not become ready — last 30 lines of .dev/api.log:"
  tail -30 .dev/api.log
  exit 1
fi
green "API ready"

# ── Demo content, through the API ───────────────────────────────────────────
# Created over HTTP rather than with INSERTs, deliberately. Going through the
# real endpoints means the demo data has the workspace owner grant D-054 issues,
# the activity and audit rows ADR-006 requires, and the outbox events the search
# projection and SSE consume. Rows inserted behind the API would look identical
# in the tables and behave differently in every feature built on top of them.
step "Creating demo content"
# Created over HTTP rather than with INSERTs, deliberately. Going through the
# real endpoints means the demo data carries the workspace owner grant D-054
# issues, the activity and audit rows ADR-006 requires, and the outbox events
# the search projection and SSE consume. Rows inserted behind the API look
# identical in the tables and behave differently in every feature built on them.
COOKIE_JAR=.dev/cookies.txt
rm -f "$COOKIE_JAR"

LOGIN=$(curl -fsS -c "$COOKIE_JAR" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${DEMO_EMAIL}\",\"password\":\"${DEMO_PASSWORD}\"}" \
  "http://127.0.0.1:${API_PORT}/api/v1/auth/login")
CSRF=$(printf '%s' "$LOGIN" | sed -n 's/.*"csrf_token":"\([^"]*\)".*/\1/p')
[[ -n "$CSRF" ]] || { red "login did not return a CSRF token"; echo "$LOGIN"; exit 1; }

# Idempotency-Key on every create. The API requires it (TF-IDM-0003) so that a
# timeout which actually succeeded cannot produce a duplicate nobody can detect,
# and a seeding script is exactly the caller that would otherwise retry blind.
api_get()  { curl -fsS -b "$COOKIE_JAR" -H "x-csrf-token: ${CSRF}" \
               ${2:+-H "x-workspace-id: $2"} "http://127.0.0.1:${API_PORT}$1"; }
api_post() { curl -fsS -b "$COOKIE_JAR" -X POST -H "x-csrf-token: ${CSRF}" \
               ${3:+-H "x-workspace-id: $3"} -H 'content-type: application/json' \
               -H "Idempotency-Key: $(uuidgen)" -d "$2" \
               "http://127.0.0.1:${API_PORT}$1"; }
first_id() { sed -n 's/.*"id":"\([^"]*\)".*/\1/p' | head -1; }

WORKSPACES=$(api_get /api/v1/workspaces)
if grep -q '"slug":"demo"' <<<"$WORKSPACES"; then
  WORKSPACE=$(printf '%s' "$WORKSPACES" | first_id)
  note "workspace demo already exists"
else
  WORKSPACE=$(api_post /api/v1/workspaces '{"name":"Demo","slug":"demo"}' | first_id)
  note "workspace demo created"
fi

PROJECTS=$(api_get /api/v1/projects "$WORKSPACE")
if grep -q '"key":"ONB"' <<<"$PROJECTS"; then
  note "projects already seeded; leaving them alone"
else
  seed_project() { # seed_project KEY NAME "title|title|..."
    local key=$1 name=$2 titles=$3
    local project
    project=$(api_post /api/v1/projects \
      "{\"name\":\"${name}\",\"key\":\"${key}\"}" "$WORKSPACE" | first_id)
    local n=0
    local IFS='|'
    for title in $titles; do
      api_post "/api/v1/projects/${project}/tasks" \
        "{\"title\":\"${title}\"}" "$WORKSPACE" >/dev/null
      n=$((n + 1))
    done
    note "${key} — ${n} tasks"
  }

  seed_project ONB "Onboarding" \
    "Set up the development environment|Read docs/02-ARCHITECTURE.md|Get database access|Pair on the outbox|Ship a first change|Write the runbook entry|Review the threat model"
  seed_project WEB "Web client" \
    "Board drag and drop|Task drawer comments|Command palette actions|Keyboard navigation audit|Bundle budget review|Dark mode|Empty states|Error boundary copy"
  seed_project API "Platform" \
    "Rate limit the write classes|Attachment virus scanning|Search relevance tuning|Outbox dead-letter alerting|Permission explain endpoint|Session revocation latency"
  seed_project OPS "Operations" \
    "Backup restore drill|Upgrade rehearsal|On-call handover doc|Capacity review"
fi

# ── The web client ──────────────────────────────────────────────────────────
step "Starting the web client on :${WEB_PORT}"
if [[ -f .dev/web.pid ]]; then pkill -P "$(cat .dev/web.pid)" 2>/dev/null || true; kill "$(cat .dev/web.pid)" 2>/dev/null || true; fi
if command -v pnpm >/dev/null 2>&1; then
  (cd webapp && pnpm install --silent >/dev/null 2>&1 || true)
  env VITE_API_URL="http://127.0.0.1:${API_PORT}" \
    nohup pnpm --dir webapp dev --port "${WEB_PORT}" > .dev/web.log 2>&1 &
  echo $! > .dev/web.pid
  sleep 3
  green "web client starting"
else
  red "pnpm is not installed — the API is up but there is no UI"
fi

cat <<EOF

$(green "TaskForge is running.")

  Web       http://127.0.0.1:${WEB_PORT}
  API       http://127.0.0.1:${API_PORT}
  Database  ${APP_DSN}

  Email     ${DEMO_EMAIL}
  Password  ${DEMO_PASSWORD}

  Logs      .dev/api.log  .dev/web.log
  Stop      scripts/dev-up.sh --down
  Rebuild   scripts/dev-up.sh --reset

EOF
