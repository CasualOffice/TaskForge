-- The assertion itself: walk one EXPLAIN (FORMAT JSON) plan and emit a line for
-- every scan node, flagging the ones that violate NFR-5.
--
--   > No user-reachable query performs a sequential scan on a tenant-scale
--   > table.  — docs/26, ADR-011
--
-- Called once per query by scripts/verify-queries.sh with the plan JSON in
-- :plan. Output is line-oriented so the driver can classify without a JSON
-- parser on the host:
--
--   SEQSCAN|<relation>|<node type>   a violation
--   INDEX|<index name>               an index the plan actually used
--
-- WHY THE FORBIDDEN SET IS RESOLVED THROUGH pg_inherits
--
-- activity_event and audit_event are range-partitioned (ADR-021). A plan never
-- names the parent — it names `activity_event_default` or, once the retention
-- worker is running, `activity_event_2026_03`. Matching on the parent's name
-- alone would make the two history tables permanently unassertable, which is the
-- quiet way this kind of gate stops covering anything. The recursion below picks
-- up every partition, at any depth, whatever it is called.

WITH RECURSIVE forbidden(oid) AS (
    SELECT c.oid
      FROM pg_class c
      JOIN pg_namespace n ON n.oid = c.relnamespace
     WHERE n.nspname = 'public'
       -- The tenant-scale tables, from tests/explain/tenant-scale-tables.txt.
       -- Everything else — permission (28 rows), workspace, workflow_status —
       -- may be scanned: reading 28 rows is the correct plan, and forbidding it
       -- would make the gate lie about what it protects.
       AND c.relname = ANY (string_to_array(:'tenant_scale', ','))
  UNION ALL
    SELECT i.inhrelid
      FROM pg_inherits i
      JOIN forbidden f ON f.oid = i.inhparent
),
forbidden_name AS (
    SELECT c.relname FROM forbidden f JOIN pg_class c ON c.oid = f.oid
),
node AS (
    SELECT n
      FROM jsonb_path_query((:'plan')::jsonb, '$.**') n
     WHERE jsonb_typeof(n) = 'object'
       AND n ? 'Node Type'
)
SELECT 'SEQSCAN|' || (n->>'Relation Name') || '|' || (n->>'Node Type')
  FROM node
 WHERE n->>'Node Type' IN ('Seq Scan', 'Parallel Seq Scan')
   AND n->>'Relation Name' IN (SELECT relname FROM forbidden_name)
UNION ALL
SELECT 'INDEX|' || (n->>'Index Name')
  FROM node
 WHERE n ? 'Index Name';
