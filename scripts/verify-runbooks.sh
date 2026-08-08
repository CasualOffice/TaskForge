#!/usr/bin/env bash
#
# Runs every query docs/50-RUNBOOKS.md marks "✅ executable" against a real
# schema, and fails naming the step whose query no longer works.
#
# WHY THIS EXISTS
#
# A runbook query is read once, in an incident, by someone under time pressure
# who cannot tell a stale query from a correct one until it errors — at the
# worst possible moment. "✅ executable" is a promise made to that person.
#
# It was already broken once. Migration 0013 moved outbox delivery state onto a
# new table and dropped three columns from outbox_event; RB-01 and RB-02 were
# written entirely against those columns. Every query in both runbooks would
# have failed, and nothing would have said so until an operator ran one during
# an outbox backlog. They were fixed by hand, by noticing. This gate is so the
# next one is noticed by CI.
#
# WHAT IT DOES NOT CHECK
#
# That the queries return USEFUL answers. It checks that they parse, that every
# table and column they name exists, and that the types line up — the failure
# mode schema drift actually produces. A query that is valid SQL and answers the
# wrong question still passes, and no gate can fix that.
#
# Each query runs inside a transaction that is rolled back, so a runbook may
# contain an UPDATE (RB-02's replay does) without this gate mutating anything.
#
#   ./scripts/verify-runbooks.sh
#
set -euo pipefail
cd "$(dirname "$0")/.."

CONTAINER=tf-runbook-verify
OWNER_DSN="${TF_VERIFY_DSN:-}"

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
step()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }

if [[ -z "$OWNER_DSN" ]]; then
  step "Starting PostgreSQL 16"
  # -v: postgres:16-alpine declares a VOLUME, so removing the container without
  # it orphans ~600 MB per run. See the same note in verify-schema.sh.
  docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true
  # No published port. This gate talks to the container through `docker exec`,
  # and publishing one only creates a collision with whatever else is running —
  # the dev compose profile already holds 55434, which is how this was found.
  docker run -d --name "$CONTAINER" \
    -e POSTGRES_USER=tf -e POSTGRES_PASSWORD=tf -e POSTGRES_DB=tf \
    postgres:16-alpine >/dev/null
  trap 'docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true' EXIT
  # -h 127.0.0.1: during initdb the entrypoint runs a temporary server on the
  # unix socket only, so a socket-based pg_isready reports ready before the real
  # server exists.
  until docker exec "$CONTAINER" pg_isready -h 127.0.0.1 -U tf -q 2>/dev/null; do sleep 1; done
  PSQL=(docker exec -i "$CONTAINER" psql -v ON_ERROR_STOP=1 -q -U tf -d tf)
else
  # CI shares one PostgreSQL service between the database gates, and
  # verify-schema.sh has already applied the migrations to it. Re-applying them
  # would fail on the first CREATE TABLE, so this gate gets its own database
  # rather than depending on the order the steps happen to run in.
  step "Creating an isolated database"
  psql -v ON_ERROR_STOP=1 -q "$OWNER_DSN" \
    -c 'DROP DATABASE IF EXISTS tf_runbooks' \
    -c 'CREATE DATABASE tf_runbooks'
  PSQL=(psql -v ON_ERROR_STOP=1 -q "${OWNER_DSN%/*}/tf_runbooks")
  echo "  ✅ tf_runbooks"
fi

step "Applying migrations"
for f in migrations/*.sql; do
  "${PSQL[@]}" < "$f" >/dev/null
done
echo "  ✅ schema ready"

# The application role must exist and be able to log in: RB-05 step 3 asks "is
# the application connecting as the right role?", and answering it requires one.
"${PSQL[@]}" -c "ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw'" >/dev/null

step "Running every query marked ✅ executable"

total=0
failed=0
while IFS= read -r -d '' n && IFS= read -r -d '' label && IFS= read -r -d '' sql && IFS= read -r -d '' _; do
  total=$((total + 1))
  # BEGIN/ROLLBACK so a runbook containing an UPDATE — RB-02's replay does —
  # cannot mutate anything here. ON_ERROR_STOP makes the whole transaction fail
  # on the first bad statement.
  # Trailing ';' removed so the two forms below can both append one.
  sql="${sql%%;}"

  if [[ "$sql" == *'$1'* ]]; then
    # The query takes bind parameters — a workspace id, a correlation id — and
    # psql cannot supply them. PREPARE validates it *fully* without values:
    # the statement is parsed and planned, every table and column resolved, and
    # the parameter types inferred. That is a stronger check than executing it
    # with an invented value, which would only prove the value cast.
    #
    # PREPARE takes a single statement. No parameterised runbook block is
    # multi-statement today; if one appears, this fails loudly with a syntax
    # error rather than skipping it.
    payload=$(printf 'BEGIN;\nPREPARE tf_runbook_check AS\n%s;\nROLLBACK;\n' "$sql")
  else
    payload=$(printf 'BEGIN;\n%s;\nROLLBACK;\n' "$sql")
  fi

  if output=$(printf '%s' "$payload" | "${PSQL[@]}" 2>&1); then
    printf '  \033[32m✅\033[0m %s\n' "$label"
  else
    failed=$((failed + 1))
    red "  ❌ $label"
    printf '%s\n' "$output" | sed 's/^/       /'
  fi
done < <(python3 scripts/runbook-queries.py)

step "Result"
if (( failed > 0 )); then
  red "
$failed of $total runbook queries no longer run against the current schema.

A runbook is read during an incident. Fix the query in docs/50-RUNBOOKS.md, or
if the capability genuinely went away, change the step's marker from
'✅ executable' to '⏳ designed' and say why."
  exit 1
fi

green "
Runbook verification passed — $total executable queries, all run."
