-- name: Task list as the filter compiler emits it, second page
-- serves: docs/26 §task — task_list_ix; docs/27 §Compilation
-- expects-index: task_list_ix
--
-- Case 03 asserts the compiled *shape*. This asserts the query the product
-- actually issues, which differs in two ways that could each change a plan:
--
--   1. The projection is the repository's explicit column list, not `id, title`
--      — `t.*` cannot be used, because `type`, `priority` and `state` come back
--      as PostgreSQL enums that no `String` decoder accepts.
--   2. The cursor's sort key is CAST on the parameter. A cursor travels as
--      text, and `timestamptz < text` is not an operator; casting the column
--      instead would work and would defeat task_list_ix, which is exactly the
--      failure this gate exists to catch.
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
   AND (TRUE)
   AND (t.updated_at, t.id) < (:'cursor_updated_at'::timestamptz, :'cursor_id'::uuid)
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
