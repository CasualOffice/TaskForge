#!/bin/sh
# Applies migrations/*.sql on first initialization, as the database OWNER.
#
# WHY THIS EXISTS
#
# Nothing applied the schema. `deploy/docker-compose.yml` brought up PostgreSQL
# and the API, `init-app-role.sh` created the two roles, the Dockerfile shipped
# `/app/migrations` into the image — and no step anywhere ran them. A stack
# started from the deployment guide came up with an empty database, an API that
# passed `/health/ready` (which is `SELECT 1`, and succeeds against no schema at
# all), and a 500 on the first real request.
#
# docs/52 §Roles already says who runs them: "POSTGRES_USER (owner) |
# migrations | needs DDL". This is that, executed rather than described.
#
# WHY AS THE OWNER, AND NOT AS THE APPLICATION
#
# The migrations create types, tables, policies and roles, and they GRANT and
# REVOKE. `taskforge_app` is deliberately a non-superuser with no DDL rights
# (migration 0012) — that is the whole mechanism behind tenant isolation and
# append-only history. An application that could migrate could also drop a
# policy.
#
# WHY AN INIT SCRIPT AND NOT A MIGRATE SERVICE
#
# It runs in the same place, at the same moment, and by the same mechanism as
# the role script beside it, which means one ordering rule instead of two: the
# `docker-entrypoint-initdb.d` numeric prefix. Roles are 10, schema is 20, and
# migrations 0012 and 0014 GRANT to roles that therefore already exist.
#
# THE LIMIT, STATED
#
# `docker-entrypoint-initdb.d` runs **only on an empty data directory**. This
# brings a NEW deployment up fully migrated; it does nothing on an upgrade.
# docs/52 §Upgrades describes expand → migrate → contract for that case, and the
# tooling for it is not built — so an operator upgrading an existing volume
# still applies migrations by hand. That is a real gap and it is written down
# rather than papered over.
set -e

# Sorted, because `for f in *.sql` is directory order on some filesystems and
# applying 0008 before 0001 fails in a way that reads like a schema bug rather
# than an ordering one.
for file in $(ls /migrations/*.sql | sort); do
    echo "applying $(basename "$file")"
    psql -v ON_ERROR_STOP=1 \
         --username "$POSTGRES_USER" \
         --dbname "$POSTGRES_DB" \
         -f "$file"
done

echo "schema applied: $(ls /migrations/*.sql | wc -l) migrations"
