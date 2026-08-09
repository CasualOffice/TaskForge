-- name: Compiled list filtered by tag and priority (C-013 grammar)
-- serves: docs/26 §task_tag — task_tag_rev_ix; docs/26 §task — task_type_prio_ix
-- expects-index: task_tag_rev_ix
--
-- `?tag=<id>&priority=>=HIGH` — two clauses, which is the ordinary case for a
-- saved view and the case a single-clause probe does not cover: the planner has
-- to choose which predicate leads, and the tag EXISTS is the selective one.
--
-- `priority >= HIGH` compares against `task_priority`, the enum, so the cast is
-- on the PARAMETER. `t.priority::text >= 'HIGH'` would read correctly, order
-- alphabetically rather than by the enum's declared order, and defeat every
-- index on the column — the exact failure the compiler's `cast_for` prevents.
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
   AND ((EXISTS (SELECT 1 FROM task_tag tt
                  WHERE tt.task_id = t.id
                    AND tt.tag_id = ANY (ARRAY[:'probe_tag']::uuid[])))
        AND t.priority >= :'probe_priority'::task_priority)
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
