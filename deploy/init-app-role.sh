#!/bin/sh
# Creates the two database roles on first initialization: the non-superuser
# application role, and the dispatcher.
#
# This exists because RLS and append-only history are enforced through role
# privileges, and NEITHER works when the application connects as a superuser:
# superusers bypass every RLS policy unconditionally, and REVOKE has no effect
# on them. See migration 0012 for the full reasoning.
#
# The dispatcher is a SECOND role, and the separation is the point. It must see
# across tenants — a background worker cannot know every workspace id — so it
# bypasses row-level security. Giving that capability to taskforge_app instead
# would hand it to every request the product serves. Migration 0014 bounds it by
# granting it nothing outside the two outbox tables.
#
# Runs once, on an empty data directory. Migrations 0012 and 0014 are idempotent
# with respect to the roles created here.
set -e

: "${TASKFORGE_DB_PASSWORD:?TASKFORGE_DB_PASSWORD must be set}"
# A separate secret, deliberately. Reusing the application password would mean
# that leaking the credential every request path uses also leaks the one that
# bypasses tenant isolation.
: "${TASKFORGE_DISPATCHER_PASSWORD:?TASKFORGE_DISPATCHER_PASSWORD must be set}"

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<-EOSQL
    DO \$\$
    BEGIN
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'taskforge_app') THEN
            CREATE ROLE taskforge_app LOGIN PASSWORD '${TASKFORGE_DB_PASSWORD}';
        ELSE
            ALTER ROLE taskforge_app LOGIN PASSWORD '${TASKFORGE_DB_PASSWORD}';
        END IF;
    END
    \$\$;
    ALTER ROLE taskforge_app NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
    GRANT CONNECT ON DATABASE "$POSTGRES_DB" TO taskforge_app;

    DO \$\$
    BEGIN
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'taskforge_dispatcher') THEN
            CREATE ROLE taskforge_dispatcher LOGIN PASSWORD '${TASKFORGE_DISPATCHER_PASSWORD}';
        ELSE
            ALTER ROLE taskforge_dispatcher LOGIN PASSWORD '${TASKFORGE_DISPATCHER_PASSWORD}';
        END IF;
    END
    \$\$;
    -- BYPASSRLS, and nothing else. Still NOSUPERUSER: a superuser would also
    -- ignore the REVOKEs that make audit history append-only.
    ALTER ROLE taskforge_dispatcher NOSUPERUSER BYPASSRLS NOCREATEDB NOCREATEROLE;
    GRANT CONNECT ON DATABASE "$POSTGRES_DB" TO taskforge_dispatcher;
EOSQL

echo "taskforge_app role ready (NOSUPERUSER, NOBYPASSRLS)"
echo "taskforge_dispatcher role ready (NOSUPERUSER, BYPASSRLS, outbox tables only)"
