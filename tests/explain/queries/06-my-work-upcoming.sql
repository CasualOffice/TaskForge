-- name: My Work · Upcoming
-- serves: docs/27 §Built-in views; docs/26 §task — task_assignee_ix
-- expects-index: task_assignee_user_ix
--
-- assignee=@me AND due_at between @tomorrow..+14d. A `between` on a datetime is
-- a range, which is why the closed operator set in docs/27 permits it.
SELECT t.id, t.number, t.title, t.due_at, t.priority, t.project_id
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
   AND t.due_at >= (:'anchor'::timestamptz + interval '1 day')
   AND t.due_at <  (:'anchor'::timestamptz + interval '15 days')
   AND EXISTS (SELECT 1 FROM task_assignee a
                WHERE a.task_id = t.id AND a.user_id = :'probe_user')
 ORDER BY t.due_at ASC, t.id ASC
 LIMIT 51
