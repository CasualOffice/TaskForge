-- name: My Work · Blocked
-- serves: docs/27 §Fields — is_blocked, backed by task_dependency_rev_ix
-- expects-index: task_dependency_rev_ix
--
-- `is_blocked` is derived, not stored. It is the one built-in view whose field
-- does not exist as a column, and it is reachable only because the reverse
-- dependency index exists: the question is "what blocks this task", which is the
-- opposite direction from the primary key.
SELECT t.id, t.number, t.title, t.updated_at
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
   AND EXISTS (SELECT 1 FROM task_assignee a
                WHERE a.task_id = t.id AND a.user_id = :'probe_user')
   AND EXISTS (SELECT 1
                 FROM task_dependency d
                 JOIN task b ON b.id = d.from_task_id
                WHERE d.to_task_id = t.id
                  AND b.state <> ALL (ARRAY['COMPLETED','CANCELED']::task_state[]))
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
