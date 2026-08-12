-- Probe constants for the query catalogue, computed rather than pasted.
--
-- Every value here mirrors the id scheme in seed.sql: a kind byte plus a
-- zero-padded ordinal. It is recomputed instead of hard-coded so a 40-element
-- accessible-project array does not have to appear literally in every query file.
--
-- Deliberately self-contained: it reads no tables and calls no seed function, so
-- the driver can \gset these before opening the scoped transaction, and so the
-- catalogue still makes sense against a corpus loaded by some other means.
--
-- The values are spliced into the queries as LITERALS, not bind parameters. That
-- is the one place this harness is more generous to the planner than production
-- is: with a literal the planner sees the actual constant and its histogram,
-- whereas a generic plan for `$1` uses average selectivity. A query that is
-- index-served here can therefore still choose a different plan under a generic
-- prepared statement. Closing that gap needs the real prepared statements, which
-- arrive with the persistence crate (docs/19); until then this is stated, not
-- hidden.

SELECT
    -- Focus tenant, and the 40 projects its probe actor can reach. docs/26
    -- §Permission filtering: `= ANY(array)` with tens of entries is the shape
    -- the compiler emits, so the assertion must use that shape too.
    '01000000-0000-7000-8000-000000000001'                        AS ws_id,
    (SELECT 'ARRAY[' || string_agg(quote_literal(
         '03000000-0000-7000-8000-' || lpad(to_hex(g), 12, '0')), ',')
         || ']::uuid[]' FROM generate_series(1, 40) g)            AS accessible_projects,
    '03000000-0000-7000-8000-000000000001'                        AS probe_project,
    -- Three visible board columns out of the workflow's six statuses.
    (SELECT 'ARRAY[' || string_agg(quote_literal(
         '09000000-0000-7000-8000-' || lpad(to_hex(g), 12, '0')), ',')
         || ']::uuid[]' FROM generate_series(1, 3) g)             AS board_statuses,
    '02000000-0000-7000-8000-000000000008'                        AS probe_user,
    '06000000-0000-7000-8000-000000000001'                        AS probe_team,
    '14000000-0000-7000-8000-000000000001'                        AS probe_task,
    -- The outbox event seed.sql writes for probe_task: kind 16 (0x10) over the
    -- same ordinal, so the two address the same aggregate. The dispatch poll's
    -- ordering predicate needs a (created_at, id) pair, and a real event id is
    -- what makes that comparison the one `claim` issues.
    '10000000-0000-7000-8000-000000000001'                        AS probe_outbox_event,
    '05000000-0000-7000-8000-000000000003'                        AS probe_tag,
    '0a000000-0000-7000-8000-000000000001'                        AS probe_milestone,
    -- The seed's anchor instant. Queries use it instead of now() so a plan is a
    -- function of the corpus alone.
    '2026-01-01 00:00:00+00'                                      AS anchor,
    '2025-12-30 12:00:00+00'                                      AS cursor_updated_at,
    '14000000-0000-7000-8000-000000000064'                        AS cursor_id,
    -- A term seeded into one task per project: the selective full-text probe.
    'zylophage'                                                   AS probe_term,
    -- The same term as the compiler builds it since D-069: a `:*` on the final
    -- token. Kept beside `probe_term` rather than replacing it so a query that
    -- wants the unprefixed form still has one.
    'zylophage:*'                                                 AS probe_term_prefix,
    'Task 42'                                                     AS probe_title_prefix,
    -- The C-013 grammar's ordered-enum probe. HIGH sits above MEDIUM in the
    -- enum's declared order, which is what `priority >= HIGH` compares against
    -- — alphabetically it would sort below it.
    'HIGH'                                                        AS probe_priority
\gset
