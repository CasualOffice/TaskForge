-- name: My Work · Today
-- serves: docs/27 §Built-in views; docs/26 §task — task_assignee_ix
-- expects-index: task_assignee_user_ix
--
-- assignee=@me AND state in (PLANNED,ACTIVE) AND due_at <= @today.
-- Many-to-many fields compile to EXISTS, not JOIN (docs/27 §Compilation), so a
-- task with two assignees appears once without a DISTINCT that would break the
-- cursor.
SELECT t.id, t.number, t.title, t.due_at, t.priority, t.project_id
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
   AND t.state = ANY (ARRAY['PLANNED','ACTIVE']::task_state[])
   AND t.due_at <= (:'anchor'::timestamptz + interval '1 day' - interval '1 microsecond')
   AND EXISTS (SELECT 1 FROM task_assignee a
                WHERE a.task_id = t.id AND a.user_id = :'probe_user')
 ORDER BY t.due_at ASC, t.id ASC
 LIMIT 51
