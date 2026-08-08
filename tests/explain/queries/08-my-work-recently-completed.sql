-- name: My Work · Recently completed
-- serves: docs/27 §Built-in views; docs/26 §task — task_mywork_ix, task_assignee_ix
-- expects-index: task_assignee_user_ix
--
-- assignee=@me AND state=COMPLETED AND updated_at > -7d.
SELECT t.id, t.number, t.title, t.updated_at, t.project_id
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND t.state = 'COMPLETED'::task_state
   AND t.updated_at > (:'anchor'::timestamptz - interval '7 days')
   AND EXISTS (SELECT 1 FROM task_assignee a
                WHERE a.task_id = t.id AND a.user_id = :'probe_user')
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
