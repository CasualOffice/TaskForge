-- name: Overdue sweep (reminder worker)
-- serves: docs/26 §task — task_due_ix (workspace_id, due_at) partial
-- expects-index: task_due_ix
--
-- A worker query, not a user query, but it runs per workspace on a schedule and
-- a scan here costs the whole fleet rather than one request. The partial index
-- (due_at IS NOT NULL AND deleted_at IS NULL) is what keeps it small: two thirds
-- of tasks in a mature workspace have no due date at all.
SELECT t.id, t.workspace_id, t.project_id, t.due_at
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.due_at IS NOT NULL
   AND t.due_at < :'anchor'::timestamptz
   AND t.deleted_at IS NULL
   AND t.state <> ALL (ARRAY['COMPLETED','CANCELED']::task_state[])
 ORDER BY t.due_at ASC
 LIMIT 500
