-- name: My Work · Overdue
-- serves: docs/27 §Built-in views; docs/26 §task — task_assignee_ix
-- expects-index: task_assignee_user_ix
--
-- assignee=@me AND state not in (COMPLETED,CANCELED) AND due_at < @today.
SELECT t.id, t.number, t.title, t.due_at, t.priority, t.project_id
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
   AND t.state <> ALL (ARRAY['COMPLETED','CANCELED']::task_state[])
   AND t.due_at < :'anchor'::timestamptz
   AND EXISTS (SELECT 1 FROM task_assignee a
                WHERE a.task_id = t.id AND a.user_id = :'probe_user')
 ORDER BY t.due_at ASC, t.id ASC
 LIMIT 51
