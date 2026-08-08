-- name: Cross-project list with the permission filter, second page
-- serves: docs/26 §task, docs/27 §Compilation
--
-- The compiled shape from docs/27: the `project_id = ANY(...)` permission filter
-- is injected by the compiler, never supplied by the caller. Kept as a separate
-- case from 02 because the multi-project form cannot use the row comparison as
-- an index qual — the planner has to combine per-project ranges — and that is a
-- different plan worth asserting on its own.
SELECT t.id, t.number, t.title, t.updated_at, t.state
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
   AND (t.updated_at, t.id) < (:'cursor_updated_at'::timestamptz, :'cursor_id'::uuid)
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
