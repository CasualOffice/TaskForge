-- name: Unassigned
-- serves: docs/27 §Built-in views — `assignee is_empty`
--
-- The negative direction of a many-to-many. NOT EXISTS is the compiled form of
-- `is_empty`; it is included because an anti-join is the case most likely to
-- degrade into a scan of `task` as the corpus grows.
SELECT t.id, t.number, t.title, t.state
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
   AND t.state = ANY (ARRAY['BACKLOG','PLANNED']::task_state[])
   AND NOT EXISTS (SELECT 1 FROM task_assignee a WHERE a.task_id = t.id)
 ORDER BY t.created_at DESC, t.id DESC
 LIMIT 51
