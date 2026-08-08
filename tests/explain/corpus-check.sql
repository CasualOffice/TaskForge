-- Corpus adequacy and coverage, run once before any plan is examined.
--
-- Two independent failure modes this answers, both of which turn the gate into
-- decoration if left unchecked:
--
--   1. THE TABLE IS TOO SMALL. PostgreSQL scans a relation of a few pages no
--      matter what indexes exist, because that genuinely is the cheaper plan. An
--      EXPLAIN suite over an empty or tiny corpus passes every assertion while
--      proving nothing. Anything under :min_rows is reported SMALL, and the
--      driver skips its queries loudly rather than passing them quietly.
--
--   2. A NEW TENANT-SCALE TABLE IS NOT LISTED. tenant-scale-tables.txt is
--      hand-maintained, and a hand-maintained list of tables silently omits the
--      next one. Any table whose row count has grown past :coverage_rows without
--      being listed is reported UNCOVERED, which fails the run.
--
-- Counts come from pg_class.reltuples, which is exact enough immediately after
-- ANALYZE and costs nothing. The thresholds carry an order-of-magnitude margin,
-- so estimate error cannot flip a verdict.
--
-- Emits one line per finding:
--   SIZE|<table>|<rows>        every listed table, for the report
--   SMALL|<table>|<rows>       listed but below :min_rows
--   UNCOVERED|<table>|<rows>   large, unlisted, and therefore ungated

WITH listed AS (
    SELECT unnest(string_to_array(:'tenant_scale', ',')) AS relname
),
-- Partitions inherit their parent's listing (docs/26: activity_event and
-- audit_event are range-partitioned monthly, ADR-021).
partitioned AS (
    SELECT c.oid, c.relname
      FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
),
child_of_listed AS (
    SELECT ch.relname
      FROM pg_inherits i
      JOIN partitioned pa ON pa.oid = i.inhparent
      JOIN partitioned ch ON ch.oid = i.inhrelid
     WHERE pa.relname IN (SELECT relname FROM listed)
),
sized AS (
    SELECT p.relname, greatest(c.reltuples, 0)::bigint AS rows
      FROM partitioned p JOIN pg_class c ON c.oid = p.oid
)
SELECT 'SIZE|' || relname || '|' || rows
  FROM sized
 WHERE relname IN (SELECT relname FROM listed)
    OR relname IN (SELECT relname FROM child_of_listed)
UNION ALL
SELECT 'SMALL|' || relname || '|' || rows
  FROM sized
 WHERE (relname IN (SELECT relname FROM listed)
     OR relname IN (SELECT relname FROM child_of_listed))
   AND rows < :min_rows
UNION ALL
SELECT 'UNCOVERED|' || relname || '|' || rows
  FROM sized
 WHERE rows >= :coverage_rows
   AND relname NOT IN (SELECT relname FROM listed)
   AND relname NOT IN (SELECT relname FROM child_of_listed)
ORDER BY 1;
