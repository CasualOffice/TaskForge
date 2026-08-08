-- name: Reported by me
-- serves: docs/26 §task — task_reporter_ix (workspace_id, reporter_id)
-- expects-index: task_reporter_ix
SELECT t.id, t.number, t.title, t.state, t.updated_at
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.reporter_id = :'probe_user'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
   AND t.state <> ALL (ARRAY['COMPLETED','CANCELED']::task_state[])
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
