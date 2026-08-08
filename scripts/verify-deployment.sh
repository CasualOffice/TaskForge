#!/usr/bin/env bash
#
# Brings up the DEPLOYMENT compose (not the dev one) and asserts that a
# real-world install is actually secure. See docs/52-DEPLOYMENT-GUIDE.md.
#
# This exists because the dangerous failures here are SILENT. A missing
# TASKFORGE_DB_PASSWORD makes the init script fail, PostgreSQL leaves a data
# directory behind, and the next start comes up healthy with no application
# role — at which point the app connects as the owner and row-level security is
# inert. Nothing looks broken. This catches exactly that.
#
set -euo pipefail
cd "$(dirname "$0")/.."

COMPOSE="deploy/docker-compose.yml"
ENVF="deploy/.env.verify"
DC="docker compose -f $COMPOSE --env-file $ENVF -p tf-deploy-verify"

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
step()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
assert_eq() {
  if [[ "$2" == "$3" ]]; then echo "  ✅ $1"; else red "  ❌ $1: expected '$2', got '$3'"; exit 1; fi
}

cleanup() { $DC down -v >/dev/null 2>&1 || true; rm -f "$ENVF"; }
trap cleanup EXIT

sed -e 's/CHANGE_ME_owner_password/ownerpw/' \
    -e 's/CHANGE_ME_app_password/apppw/' \
    -e 's/CHANGE_ME_generate_with_openssl_rand_base64_48/dGVzdC1vbmx5LW5vdC1hLXJlYWwtc2VjcmV0LTAwMDAwMDA=/' \
    deploy/.env.example > "$ENVF"

step "Compose refuses to start unconfigured"
if docker compose -f "$COMPOSE" config >/dev/null 2>&1; then
  red "  ❌ started with no configuration"; exit 1
fi
echo "  ✅ required variables are enforced"

step "Bringing up PostgreSQL from the deployment compose"
$DC up -d postgres >/dev/null
# -h 127.0.0.1 is load-bearing: during initdb the entrypoint runs a temporary
# server on the unix socket ONLY, so a socket-based pg_isready reports ready
# before the real server exists. Without it this gate is flaky — it passed one
# CI run and failed the next with "server closed the connection unexpectedly",
# 120 ms after reporting healthy.
until $DC exec -T postgres pg_isready -h 127.0.0.1 -U taskforge_owner -q 2>/dev/null; do sleep 1; done
echo "  ✅ healthy"

step "The application role is created and correctly constrained"
# Booleans concatenated to text render as false/true (not f/t, which is only
# the column display form).
assert_eq "role exists, is NOT a superuser, does NOT bypass RLS, can log in" \
  "false|false|true" \
  "$($DC exec -T postgres psql -U taskforge_owner -d taskforge -tAc \
      "select rolsuper||'|'||rolbypassrls||'|'||rolcanlogin from pg_roles where rolname='taskforge_app'" | tr -d '[:space:]')"

step "Applying migrations"
for f in migrations/*.sql; do
  $DC exec -T postgres psql -U taskforge_owner -d taskforge -v ON_ERROR_STOP=1 -q < "$f" >/dev/null
done
assert_eq "row-level security is enabled on every tenant table" "30" \
  "$($DC exec -T postgres psql -U taskforge_owner -d taskforge -tAc \
      "select count(*) from pg_class c join pg_namespace n on n.oid=c.relnamespace
        where n.nspname='public' and c.relrowsecurity" | tr -d '[:space:]')"

step "A real application connection is genuinely constrained"
assert_eq "unscoped session sees no tenant data" "0" \
  "$($DC exec -T -e PGPASSWORD=apppw postgres psql -U taskforge_app -h 127.0.0.1 -d taskforge \
      -tAc 'select count(*) from task' | tr -d '[:space:]')"

if $DC exec -T -e PGPASSWORD=apppw postgres psql -U taskforge_app -h 127.0.0.1 -d taskforge \
     -tAc "update audit_event set event_type='x'" >/dev/null 2>&1; then
  red "  ❌ audit history is rewritable by the application"; exit 1
fi
echo "  ✅ audit history rejects UPDATE"

green "
Deployment verification passed."
