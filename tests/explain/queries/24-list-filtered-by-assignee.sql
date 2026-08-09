-- name: Compiled list filtered by assignee (C-013 grammar)
-- serves: docs/26 §task_assignee — task_assignee_user_ix; docs/27 §Compilation
-- expects-index: task_assignee_user_ix
--
-- `?assignee=@me` after symbol resolution. The clause is an EXISTS and not a
-- JOIN on purpose (docs/27): a task with two assignees would appear twice under
-- a join, which forces DISTINCT, which breaks keyset pagination because
-- (updated_at, id) stops being a total order over the result set.
--
-- The subquery carries `a.workspace_id` as well as `a.user_id` because
-- task_assignee_user_ix is `(user_id, workspace_id)` — the tenant column is in
-- the index, so including it keeps the probe index-only rather than sending it
-- to the heap to re-check a column the index already has.
SELECT t.id, t.workspace_id, t.project_id, t.number, t.title, t.description,
       t.type::text AS "type", t.priority::text AS priority, t.status_id,
       t.state::text AS state,
       t.reporter_id, t.environment_id, t.milestone_id, t.parent_id,
       t.start_at, t.due_at, t.position, t.created_at, t.created_by,
       t.updated_at, t.updated_by, t.version, t.archived_at
  FROM task t
 WHERE t.workspace_id = :'ws_id'
   AND t.project_id = ANY (:accessible_projects)
   AND t.deleted_at IS NULL
   AND (EXISTS (SELECT 1 FROM task_assignee a
                 WHERE a.task_id = t.id
                   AND a.user_id = :'probe_user'::uuid))
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
