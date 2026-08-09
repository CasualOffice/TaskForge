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
  # The API and the web server are started with `setsid` below so they survive
  # this script exiting; killing the process group is how they are reclaimed.
  for pidfile in .dev/api.pid .dev/web.pid; do
    if [[ -f "$pidfile" ]]; then
      kill -- "-$(cat "$pidfile")" 2>/dev/null || true
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

# The API refuses to start as a superuser (see crates/casual-task-api/src/main.rs
# and migration 0012), and it is right to: a superuser bypasses every RLS policy,
# so tenant isolation would be silently inert in the very environment where a
# developer decides the product works. Connecting as taskforge_app here is not
# ceremony; it is what makes the dev stack behave like production.
psql_owner -q -c "ALTER ROLE taskforge_app WITH LOGIN PASSWORD '${DB_PASSWORD}';" >/dev/null
psql_owner -q -c "ALTER ROLE taskforge_dispatcher WITH LOGIN PASSWORD '${DB_PASSWORD}';" >/dev/null

APP_DSN="postgres://taskforge_app:${DB_PASSWORD}@127.0.0.1:${PGPORT}/tf"

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
if [[ -f .dev/api.pid ]]; then kill -- "-$(cat .dev/api.pid)" 2>/dev/null || true; fi
# TF_SECRET_KEY is fixed so sessions survive a restart of this script — a
# developer logged out by every rebuild stops using the dev stack.
env DATABASE_URL="$APP_DSN" \
    TF_BIND_ADDR="127.0.0.1:${API_PORT}" \
    TF_SECRET_KEY="dev-only-secret-key-not-for-any-real-deployment" \
    TF_PUBLIC_URL="http://127.0.0.1:${WEB_PORT}" \
  setsid ./target/release/casual-task-api > .dev/api.log 2>&1 &
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
COOKIE_JAR=.dev/cookies.txt
rm -f "$COOKIE_JAR"

LOGIN=$(curl -fsS -c "$COOKIE_JAR" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${DEMO_EMAIL}\",\"password\":\"${DEMO_PASSWORD}\"}" \
  "http://127.0.0.1:${API_PORT}/api/v1/auth/login")
CSRF=$(printf '%s' "$LOGIN" | sed -n 's/.*"csrf_token":"\([^"]*\)".*/\1/p')
[[ -n "$CSRF" ]] || { red "login did not return a CSRF token"; echo "$LOGIN"; exit 1; }

api() { # api METHOD PATH [JSON] [WORKSPACE]
  local method=$1 path=$2 body=${3:-} workspace=${4:-}
  local args=(-fsS -b "$COOKIE_JAR" -c "$COOKIE_JAR" -X "$method"
              -H "x-csrf-token: ${CSRF}" -H 'content-type: application/json')
  [[ -n "$workspace" ]] && args+=(-H "x-workspace-id: ${workspace}")
  [[ -n "$body" ]] && args+=(-d "$body")
  curl "${args[@]}" "http://127.0.0.1:${API_PORT}${path}"
}

id_of() { sed -n 's/.*"id":"\([^"]*\)".*/\1/p' <<<"$1"; }

WORKSPACES=$(api GET /api/v1/workspaces)
if grep -q '"slug":"demo"' <<<"$WORKSPACES"; then
  note "the demo workspace already exists; leaving it alone"
else
  WORKSPACE=$(id_of "$(api POST /api/v1/workspaces '{"name":"Demo","slug":"demo"}')")
  PROJECT=$(id_of "$(api POST /api/v1/projects \
    '{"name":"Onboarding","key":"ONB"}' "$WORKSPACE")")
  for title in \
    "Set up the development environment" \
    "Read docs/02-ARCHITECTURE.md" \
    "Draw the board" \
    "Wire the task drawer" \
    "Ship something a user can open"
  do
    api POST "/api/v1/projects/${PROJECT}/tasks" \
      "{\"title\":$(printf '%s' "$title" | sed 's/"/\\"/g; s/^/"/; s/$/"/')}" \
      "$WORKSPACE" >/dev/null
  done
  note "workspace demo · project ONB · 5 tasks"
fi

# ── The web client ──────────────────────────────────────────────────────────
step "Starting the web client on :${WEB_PORT}"
if [[ -f .dev/web.pid ]]; then kill -- "-$(cat .dev/web.pid)" 2>/dev/null || true; fi
if command -v pnpm >/dev/null 2>&1; then
  (cd webapp && pnpm install --silent >/dev/null 2>&1 || true)
  env VITE_API_URL="http://127.0.0.1:${API_PORT}" \
    setsid pnpm --dir webapp dev --port "${WEB_PORT}" > .dev/web.log 2>&1 &
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
