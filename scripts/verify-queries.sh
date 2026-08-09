#!/usr/bin/env bash
#
# The EXPLAIN gate. Applies every migration to a clean PostgreSQL 16, loads a
# deterministic corpus, and asserts the governing rule from
# docs/26-SEARCH-INDEXING-AND-QUERY.md (NFR-5, ADR-011):
#
#   > No user-reachable query performs a sequential scan on a tenant-scale table.
#
# Each query in tests/explain/queries/ is a real read path the product will
# issue. For each, the plan is taken with EXPLAIN (FORMAT JSON) and every scan
# node is checked against tests/explain/tenant-scale-tables.txt. A sequential
# scan on one of those tables fails the run and prints the offending query, the
# offending node, and the full plan.
#
# WHY THERE IS A CORPUS AND WHY IT MATTERS MORE THAN THE ASSERTIONS
#
# Against an empty database every one of these assertions passes, because
# PostgreSQL scans a zero-page relation regardless of its indexes — and a gate
# that passes without examining an index is worse than no gate, since it reports
# a guarantee nobody has. So this script refuses to be quiet about it: any
# tenant-scale table below TF_MIN_ROWS turns its queries into loud SKIPs and the
# run exits non-zero.
#
# QUERIES ARE PLANNED AS taskforge_app, NOT AS THE OWNER
#
# The application connects as a non-superuser and every read carries the RLS
# predicate `workspace_id = NULLIF(current_setting('taskforge.workspace_id',
# true), '')::uuid` (migration 0010). That predicate changes plans — it is a
# stable expression the planner cannot fold, and it competes with the query's own
# quals for index usage. Planning as the owner would skip it entirely and gate a
# query the product never issues.
#
# Locally it starts its own container. In CI it uses the `postgres` service
# (set TF_VERIFY_DSN). See docs/15-CI-AND-RELEASE-GATES.md.
#
#   ./scripts/verify-queries.sh                 # start a container, seed, assert
#   ./scripts/verify-queries.sh --data-loaded   # assert against whatever is there
#   ./scripts/verify-queries.sh --allow-skip    # do not fail the run on skips
#
# --data-loaded asserts against the corpus already in the target database instead
# of loading seed.sql. The probe constants in tests/explain/probes.sql follow
# seed.sql's id scheme, so against unrelated data they address rows that do not
# exist — the plans are still real plans, but they are planned for an empty
# result and are worth much less. Migrations are still applied if the schema is
# absent, which is what makes `--data-loaded` on a fresh database the way to see
# the empty-corpus behaviour: every assertion SKIPs, and the run exits 2.
#
# Exit codes:  0 every query index-served · 1 a sequential scan · 2 corpus too
# small to prove anything · 3 usage error.
#
set -euo pipefail
cd "$(dirname "$0")/.."

# An explicit template rather than `mktemp -t tf-explain`: BSD/macOS treats the
# argument to -t as a prefix and appends its own X's, GNU/Linux treats it as a
# template and rejects one without them. The macOS form passed every local run
# and failed on the first CI run with "too few X's in template".
ERRLOG=$(mktemp "${TMPDIR:-/tmp}/tf-explain.XXXXXX")
trap 'rm -f "$ERRLOG"' EXIT

# Unique per run. A fixed name meant two concurrent runs shared one container:
# the second one's `docker rm -f` destroyed the first one's database mid-gate,
# and the first then reported "sequential scan" and `role does not exist` for
# every remaining probe. That reads as a real plan regression, which is the
# worst possible way for a race to present itself. $$ is the pid; the random
# suffix covers the case where a container outlives the pid that made it.
CONTAINER="tf-query-verify-$$-${RANDOM}"
OWNER_DSN="${TF_VERIFY_DSN:-}"
OWNED_CONTAINER=0
DATA_LOADED=0
ALLOW_SKIP=0

# Below this, a sequential scan may genuinely be the cheapest plan and an
# assertion proves nothing. Roughly 45 pages of `task`.
MIN_ROWS="${TF_MIN_ROWS:-5000}"
# Above this, a table is tenant-scale and must appear in tenant-scale-tables.txt.
COVERAGE_ROWS="${TF_COVERAGE_ROWS:-20000}"

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
amber() { printf '\033[33m%s\033[0m\n' "$*"; }
step()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --data-loaded) DATA_LOADED=1 ;;
    --allow-skip)  ALLOW_SKIP=1 ;;
    -h|--help)     awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"
                   exit 0 ;;
    *)             red "unknown argument: $1"; exit 3 ;;
  esac
  shift
done

if [[ -z "$OWNER_DSN" ]]; then
  step "Starting PostgreSQL 16"
  # -v matters: postgres:16-alpine declares VOLUME /var/lib/postgresql/data, so
  # every run creates an anonymous volume. Removing the container without it
  # orphans ~600 MB per run, which accumulates silently on a developer machine
  # until the disk fills. (It did.)
  docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true
  # No published host port on purpose: every connection below goes through
  # `docker exec`, so binding one would only create a conflict with a concurrent
  # verify-schema.sh run or a leftover container, and that conflict looks
  # nothing like a query-plan problem when it happens.
  docker run -d --name "$CONTAINER" \
    -e POSTGRES_USER=tf -e POSTGRES_PASSWORD=tf -e POSTGRES_DB=tf \
    postgres:16-alpine >/dev/null
  OWNED_CONTAINER=1
  trap 'docker rm -f -v "$CONTAINER" >/dev/null 2>&1 || true; rm -f "$ERRLOG"' EXIT
  # -h 127.0.0.1 is load-bearing. During initdb the entrypoint runs a temporary
  # server that listens on the unix socket ONLY, so a socket-based pg_isready
  # reports ready seconds before the real server exists, and the first migration
  # then fails against a server that is about to be restarted.
  for _ in $(seq 1 60); do
    docker exec "$CONTAINER" pg_isready -h 127.0.0.1 -U tf -q 2>/dev/null && break
    sleep 1
  done
  if ! docker exec "$CONTAINER" pg_isready -h 127.0.0.1 -U tf -q 2>/dev/null; then
    red "PostgreSQL did not become ready within 60s"; exit 3
  fi
fi

# psql is not assumed to be installed on the host; route through the container
# when we own one, otherwise use the host's psql against the CI service.
if [[ $OWNED_CONTAINER -eq 1 ]]; then
  psql_owner() { docker exec -i "$CONTAINER" psql -U tf -d tf "$@"; }
  psql_app()   { docker exec -i -e PGPASSWORD=apppw "$CONTAINER" \
                   psql -U taskforge_app -h 127.0.0.1 -d tf "$@"; }
else
  # CI: an external server (TF_VERIFY_DSN). The app DSN differs from the owner
  # DSN in role only — and that difference is what puts RLS in the plans.
  TF_HOST="${TF_VERIFY_HOST:-127.0.0.1}"
  TF_PORT="${TF_VERIFY_PORT:-5432}"
  TF_DB="${TF_VERIFY_DB:-tf}"
  psql_owner() { psql "$OWNER_DSN" "$@"; }
  psql_app()   { PGPASSWORD=apppw psql \
                   -h "$TF_HOST" -p "$TF_PORT" -U taskforge_app -d "$TF_DB" "$@"; }
fi

# Migrations are not idempotent — CREATE TYPE and CREATE TABLE both fail on a
# second pass — so they run only against a database with no schema yet. That is
# what makes --data-loaded usable against a CI service a previous step already
# migrated AND against a container this script started.
schema_present=$(psql_owner -tAX -c "SELECT to_regclass('public.task') IS NOT NULL" \
                   2>/dev/null || echo f)

if [[ "$schema_present" == "t" ]]; then
  step "Schema already present — migrations not re-applied"
else
  step "Applying migrations"
  for f in migrations/*.sql; do
    if psql_owner -v ON_ERROR_STOP=1 -q < "$f" >/dev/null 2>&1; then
      echo "  ✅ $(basename "$f")"
    else
      red "  ❌ $(basename "$f")"
      psql_owner -v ON_ERROR_STOP=1 -q < "$f" 2>&1 | head -15
      exit 1
    fi
  done
fi

# Migration 0012 creates the role NOLOGIN with no password, on purpose: the
# deployment assigns credentials. This gate is a deployment.
psql_owner -q -c "ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw';" >/dev/null

if [[ $DATA_LOADED -eq 1 ]]; then
  step "Using the corpus already present (--data-loaded)"
  # Statistics, not rows, are what the planner reads. Refreshing them is cheap
  # and removes the commonest cause of a nonsense plan in a borrowed database.
  psql_owner -q -c 'ANALYZE;' >/dev/null
else
  # Seeding twice would double every derived table and destroy the determinism
  # the whole gate rests on.
  existing=$(psql_owner -tAX -c 'SELECT count(*) FROM task' 2>/dev/null || echo 0)
  if [[ "${existing:-0}" -gt 0 ]]; then
    red "  ❌ target database already contains $existing tasks"
    echo "     Use --data-loaded to assert against it, or point at a clean database."
    exit 3
  fi
  step "Loading the planning corpus (tests/explain/seed.sql)"
  psql_owner -v ON_ERROR_STOP=1 -q < tests/explain/seed.sql
fi

# ---------------------------------------------------------------------------
# Corpus adequacy and coverage
# ---------------------------------------------------------------------------
TENANT_SCALE=$(grep -Ev '^\s*(#|$)' tests/explain/tenant-scale-tables.txt | paste -sd, -)

step "Corpus adequacy"
corpus=$(psql_app -tAX -q \
           -v tenant_scale="$TENANT_SCALE" \
           -v min_rows="$MIN_ROWS" \
           -v coverage_rows="$COVERAGE_ROWS" \
           -f - < tests/explain/corpus-check.sql)

SMALL_TABLES=$(printf '%s\n' "$corpus" | awk -F'|' '$1=="SMALL"{print $2}')
UNCOVERED=$(printf '%s\n' "$corpus" | awk -F'|' '$1=="UNCOVERED"{printf "%s (%s rows)\n", $2, $3}')

printf '%s\n' "$corpus" | awk -F'|' '$1=="SIZE"{printf "  %-24s %10s rows\n", $2, $3}' | sort

if [[ -n "$UNCOVERED" ]]; then
  red "
  ❌ Tenant-scale tables missing from tests/explain/tenant-scale-tables.txt:"
  printf '     %s\n' $UNCOVERED
  echo "     A table this large is a query-debt liability. Add it to the list and
     add the query that reads it to tests/explain/queries/ (AGENTS.md:
     \"No query path without its index\")."
  exit 1
fi

if [[ -n "$SMALL_TABLES" ]]; then
  amber "
  ⚠  Below $MIN_ROWS rows — the planner may prefer a sequential scan here no
     matter what indexes exist, so assertions touching these tables prove
     nothing and will be SKIPPED, not passed:"
  printf '     %s\n' $SMALL_TABLES
fi

# ---------------------------------------------------------------------------
# The assertions
# ---------------------------------------------------------------------------
step "EXPLAIN assertions (planned as taskforge_app, RLS applied)"

PASSED=0; FAILED=0; SKIPPED=0
FAILED_NAMES=(); SKIPPED_NAMES=(); MISSING_INDEX=()

# Builds the psql input for one query: probe constants, a tenant-scoped
# transaction, then the query under EXPLAIN. Everything goes through stdin
# because psql may be running inside the container and cannot see these paths.
compose() { # $1 = query file, $2 = EXPLAIN options
  cat tests/explain/probes.sql
  echo "BEGIN;"
  echo "SET LOCAL taskforge.workspace_id = :'ws_id';"
  echo "EXPLAIN ($2)"
  cat "$1"
  echo ";"
  echo "COMMIT;"
}

# Runs the assertion over one query and leaves the classified node lines in
# $NODES. Kept as a function so the negative control goes through exactly the
# same path as the catalogue — a self-check that tested a different code path
# would be testing nothing.
analyse() { # $1 = query file
  local plan
  plan=$(compose "$1" "FORMAT JSON" | psql_app -tAX -q -v ON_ERROR_STOP=1)
  NODES=$(psql_app -tAX -q -v ON_ERROR_STOP=1 \
            -v tenant_scale="$TENANT_SCALE" -v plan="$plan" \
            -f - < tests/explain/assert-no-seq-scan.sql)
}

# The detector must be observed to fire before its silence means anything.
if analyse tests/explain/negative-control.sql &&
   printf '%s\n' "$NODES" | grep -q '^SEQSCAN|task|'; then
  green "  ✅ detector self-check — the negative control was caught"
else
  red "  ❌ detector self-check FAILED"
  echo "     tests/explain/negative-control.sql is an unindexed scan of \`task\` and"
  echo "     was NOT flagged. Every result below is therefore meaningless: the"
  echo "     assertion, not the queries, is what is broken."
  exit 1
fi

for q in tests/explain/queries/*.sql; do
  base=$(basename "$q" .sql)
  name=$(sed -n 's/^-- name: //p' "$q" | head -1)
  want_index=$(sed -n 's/^-- expects-index: //p' "$q" | head -1)
  label="${name:-$base}"

  if ! analyse "$q" 2>"$ERRLOG"; then
    red "  ❌ $label"
    echo "     EXPLAIN failed for tests/explain/queries/$base.sql:"
    sed 's/^/     /' "$ERRLOG"
    FAILED=$((FAILED + 1)); FAILED_NAMES+=("$base"); continue
  fi
  nodes="$NODES"

  # sort -u: one relation scanned twice in a plan is one finding, not two.
  offenders=$(printf '%s\n' "$nodes" | awk -F'|' '$1=="SEQSCAN"{print $2"|"$3}' | sort -u)
  indexes=$(printf '%s\n' "$nodes" | awk -F'|' '$1=="INDEX"{print $2}' | sort -u \
              | paste -sd, - | sed 's/,/, /g')

  # An offender on a table too small to plan realistically is not evidence of a
  # missing index — it is evidence of a missing corpus. Those are separated so a
  # thin database cannot masquerade as a passing gate OR as a real defect.
  # Newline-separated, never space-separated: a plan node type is "Seq Scan",
  # and word-splitting it turns one violation into two nonsense lines.
  small_offender=""
  real_offender=""
  while IFS= read -r o; do
    [[ -z "$o" ]] && continue
    rel="${o%%|*}"
    if printf '%s\n' "$SMALL_TABLES" | grep -qx "$rel"; then
      small_offender+="$o"$'\n'
    else
      real_offender+="$o"$'\n'
    fi
  done <<< "$offenders"

  if [[ -n "$real_offender" ]]; then
    red "  ❌ $label"
    echo "     tests/explain/queries/$base.sql"
    while IFS= read -r o; do
      [[ -z "$o" ]] && continue
      red "     SEQUENTIAL SCAN on ${o%%|*}  (plan node: ${o##*|})"
    done <<< "$real_offender"
    echo "     docs/26: no user-reachable query performs a sequential scan on a"
    echo "     tenant-scale table. Either the query is wrong or its index is missing."
    echo "     ---- query ----"
    grep -v '^--' "$q" | sed 's/^/     /'
    echo "     ---- plan ----"
    compose "$q" "COSTS ON, VERBOSE OFF" | psql_app -tAX -q 2>&1 | sed 's/^/     /'
    FAILED=$((FAILED + 1)); FAILED_NAMES+=("$base")
  elif [[ -n "$small_offender" ]]; then
    amber "  ⏭  $label — SKIPPED"
    while IFS= read -r o; do
      [[ -z "$o" ]] && continue
      echo "     scanned ${o%%|*}, which holds fewer than $MIN_ROWS rows; the plan"
      echo "     says nothing about the index at production scale."
    done <<< "$small_offender"
    SKIPPED=$((SKIPPED + 1)); SKIPPED_NAMES+=("$base")
  else
    green "  ✅ $label"
    [[ -n "$indexes" ]] && echo "     $indexes"
    PASSED=$((PASSED + 1))
    # Advisory only. Which index the planner picks legitimately varies with
    # corpus size, and failing on it would make the gate brittle in exactly the
    # way that gets gates disabled. The no-scan rule is the contract; this line
    # is a drift signal for review.
    if [[ -n "$want_index" ]] && ! printf '%s\n' "$nodes" | grep -qx "INDEX|$want_index"; then
      amber "     note: docs/26 names $want_index for this query; the planner chose otherwise"
      MISSING_INDEX+=("$base: $want_index")
    fi
  fi
done

# ---------------------------------------------------------------------------
step "Result"
echo "  $PASSED index-served · $FAILED sequential scans · $SKIPPED skipped"

if [[ ${#MISSING_INDEX[@]} -gt 0 ]]; then
  amber "
  Advisory — the index docs/26 names was not the one chosen (not a failure):"
  printf '    %s\n' "${MISSING_INDEX[@]}"
fi

if [[ $FAILED -gt 0 ]]; then
  red "
Query verification FAILED — sequential scan on a tenant-scale table:"
  printf '    %s\n' "${FAILED_NAMES[@]}"
  exit 1
fi

if [[ $SKIPPED -gt 0 ]]; then
  amber "
Query verification INCONCLUSIVE — $SKIPPED assertion(s) had no corpus to plan
against and were skipped, not passed:"
  printf '    %s\n' "${SKIPPED_NAMES[@]}"
  echo "
A gate that passes without examining an index reports a guarantee nobody has,
so this exits non-zero. Load a corpus, or pass --allow-skip if you are running
this locally and know why."
  if [[ $ALLOW_SKIP -eq 1 ]]; then exit 0; fi
  exit 2
fi

green "
Query verification passed — $PASSED queries, no sequential scan on a tenant-scale table."
