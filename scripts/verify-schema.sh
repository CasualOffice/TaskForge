#!/usr/bin/env bash
#
# Applies every migration to a clean PostgreSQL 16 and asserts the schema
# invariants — including the two that only hold when connected as the
# non-superuser application role: tenant isolation and append-only history.
#
# Locally it starts its own container. In CI it uses the `postgres` service
# (set TF_VERIFY_DSN). See docs/15-CI-AND-RELEASE-GATES.md.
#
#   ./scripts/verify-schema.sh
#
set -euo pipefail
cd "$(dirname "$0")/.."

# Unique per run — see the same comment in verify-queries.sh. A fixed name let
# one run's `docker rm -f` destroy another run's database mid-gate, and the
# victim reported it as a schema failure rather than as a collision.
CONTAINER="tf-schema-verify-$$-${RANDOM}"
OWNER_DSN="${TF_VERIFY_DSN:-}"
OWNED_CONTAINER=0

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
step()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }

if [[ -z "$OWNER_DSN" ]]; then
  step "Starting PostgreSQL 16"
  # -v matters: postgres:16-alpine declares VOLUME /var/lib/postgresql/data, so
  # every run creates an anonymous volume. Removing the container without it
  # orphans ~600 MB per run, which accumulates silently on a developer machine
  # until the disk fills. (It did.)
  docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true
  docker run -d --name "$CONTAINER" \
    -e POSTGRES_USER=tf -e POSTGRES_PASSWORD=tf -e POSTGRES_DB=tf \
    postgres:16-alpine >/dev/null
  OWNED_CONTAINER=1
  trap 'docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true' EXIT
  # -h 127.0.0.1 is load-bearing. During initdb the entrypoint runs a temporary
  # server that listens on the unix socket ONLY, so a socket-based pg_isready
  # reports ready seconds before the real server exists, and the next command
  # then hits a server that is about to be restarted. scripts/verify-queries.sh
  # already guards this; verify-deployment.sh did not, and CI caught it as an
  # intermittent "server closed the connection unexpectedly".
  until docker exec "$CONTAINER" pg_isready -h 127.0.0.1 -U tf -q 2>/dev/null; do sleep 1; done
  # No published host port, and so no owner DSN: every query below goes through
  # `docker exec`, so the port only ever produced a bind conflict with a
  # concurrent run — and a bind conflict looks nothing like a schema failure
  # when it lands. `psql_owner` covers this branch already.
fi

# psql is not assumed to be installed on the host; route through the container
# when we own one, otherwise use the host's psql against the CI service.
if [[ $OWNED_CONTAINER -eq 1 ]]; then
  psql_owner() { docker exec -i "$CONTAINER" psql -U tf -d tf "$@"; }
  psql_app()   { docker exec -i -e PGPASSWORD=apppw "$CONTAINER" \
                   psql -U taskforge_app -h 127.0.0.1 -d tf "$@"; }
else
  # CI: an external server (TF_VERIFY_DSN). The app DSN must differ from the
  # owner DSN in role only — that difference is what the isolation assertions
  # are actually testing.
  TF_HOST="${TF_VERIFY_HOST:-127.0.0.1}"
  TF_PORT="${TF_VERIFY_PORT:-5432}"
  TF_DB="${TF_VERIFY_DB:-tf}"
  psql_owner() { psql "$OWNER_DSN" "$@"; }
  psql_app()   { PGPASSWORD=apppw psql \
                   -h "$TF_HOST" -p "$TF_PORT" -U taskforge_app -d "$TF_DB" "$@"; }
fi

step "Applying migrations"
for f in migrations/*.sql; do
  if psql_owner -v ON_ERROR_STOP=1 -q < "$f" >/dev/null 2>&1; then
    echo "  ✅ $(basename "$f")"
  else
    red "  ❌ $(basename "$f")"; psql_owner -v ON_ERROR_STOP=1 -q < "$f" 2>&1 | head -15; exit 1
  fi
done

psql_owner -q -c "ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw';" >/dev/null

step "Structural assertions"
psql_owner -v ON_ERROR_STOP=1 -q -f - < tests/schema/assertions.sql

step "Tenant isolation (as taskforge_app — non-superuser, RLS applies)"
psql_owner -q -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
INSERT INTO workspace (id,name,slug) VALUES
 ('11111111-1111-7111-8111-111111111111','Alpha','alpha'),
 ('22222222-2222-7222-8222-222222222222','Beta','beta');
INSERT INTO user_account (id,email,display_name)
 VALUES ('aaaaaaaa-0000-7000-8000-000000000001','a@x.test','A');
INSERT INTO workspace_membership (workspace_id,user_id,member_type) VALUES
 ('11111111-1111-7111-8111-111111111111','aaaaaaaa-0000-7000-8000-000000000001','MEMBER'),
 ('22222222-2222-7222-8222-222222222222','aaaaaaaa-0000-7000-8000-000000000001','MEMBER');
SQL

assert_eq() { # label, expected, actual
  if [[ "$2" == "$3" ]]; then echo "  ✅ $1"; else red "  ❌ $1: expected '$2', got '$3'"; exit 1; fi
}

# psql -c with several statements echoes a command tag per statement
# (BEGIN / SET / COMMIT). Keep only the numeric result row.
only_number() { grep -Ex '[0-9]+' | head -1; }

# Fails closed with no scope set.
assert_eq "unscoped session sees nothing" "0" \
  "$(psql_app -tAc 'SELECT count(*) FROM workspace_membership')"

# Sees exactly its own tenant.
assert_eq "scoped session sees only its tenant" "1" \
  "$(psql_app -tAc "BEGIN; SET LOCAL taskforge.workspace_id='11111111-1111-7111-8111-111111111111'; SELECT count(*) FROM workspace_membership; COMMIT;" | only_number)"

# The NULLIF guard: a reused pooled connection must not error.
assert_eq "no pool bleed and no error after COMMIT" "0" \
  "$(psql_app -tAc 'SELECT count(*) FROM workspace_membership')"

# The one that matters: no cross-tenant row, ever.
assert_eq "other tenant's rows never visible" "0" \
  "$(psql_app -tAc "BEGIN; SET LOCAL taskforge.workspace_id='22222222-2222-7222-8222-222222222222'; SELECT count(*) FILTER (WHERE workspace_id='11111111-1111-7111-8111-111111111111') FROM workspace_membership; COMMIT;" | only_number)"

step "Append-only history (enforced by GRANT, not by convention)"
for tbl in audit_event activity_event; do
  for op in "UPDATE $tbl SET event_type='x'" "DELETE FROM $tbl"; do
    if psql_app -tAc "$op" >/dev/null 2>&1; then
      red "  ❌ ${op%% *} on $tbl was permitted — history is rewritable"; exit 1
    fi
  done
  echo "  ✅ $tbl rejects UPDATE and DELETE"
done

green "
Schema verification passed."
