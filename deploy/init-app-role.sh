#!/bin/sh
# Creates the non-superuser application role on first database initialization.
#
# This exists because RLS and append-only history are enforced through role
# privileges, and NEITHER works when the application connects as a superuser:
# superusers bypass every RLS policy unconditionally, and REVOKE has no effect
# on them. See migration 0012 for the full reasoning.
#
# Runs once, on an empty data directory. Migration 0012 is idempotent and will
# not conflict with the role created here.
set -e

: "${TASKFORGE_DB_PASSWORD:?TASKFORGE_DB_PASSWORD must be set}"

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
EOSQL

echo "taskforge_app role ready (NOSUPERUSER, NOBYPASSRLS)"
