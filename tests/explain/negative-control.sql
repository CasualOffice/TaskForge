-- The negative control: a query that MUST be caught.
--
-- A detector that has never been observed to fire is not known to work. Every
-- run plans this query first and requires the assertion to flag it; if the
-- flagging stops happening — a changed EXPLAIN format, a broken jsonb path, a
-- typo in tenant-scale-tables.txt — the gate reports that it is broken instead
-- of quietly passing everything forever.
--
-- `description` is deliberately not in the filterable field set (docs/27
-- §Fields), has no index by design, and a leading-wildcard match cannot use one
-- anyway. This is exactly the shape docs/26 exists to prevent: a filter that
-- works on five hundred tasks and is a full scan at two million.
--
-- It is never part of the catalogue and no endpoint may issue it.
SELECT count(*)
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.description LIKE '%needle%'
