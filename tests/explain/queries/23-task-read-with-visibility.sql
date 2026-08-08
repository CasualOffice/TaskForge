-- name: Read one task, with the project visibility predicate
-- serves: docs/26 §task — the primary key, plus project_membership_user_ix
-- expects-index: task_pkey
--
-- GET /api/v1/tasks/{id}. The visibility rule is joined on rather than checked
-- afterwards, so a task in a project the actor cannot see produces **no row** —
-- which is what makes "404, never 403" structural instead of a rule in a
-- handler (docs/04).
--
-- The join is the part worth planning: an id lookup that degenerated into a
-- scan of `task` because the project predicate could not be pushed down would
-- make the single most linked-to URL in the product a full scan.
SELECT t.id, t.workspace_id, t.project_id, t.number, t.title, t.description,
       t.type::text AS "type", t.priority::text AS priority, t.status_id,
       t.state::text AS state,
       t.reporter_id, t.environment_id, t.milestone_id, t.parent_id,
       t.start_at, t.due_at, t.position, t.created_at, t.created_by,
       t.updated_at, t.updated_by, t.version, t.archived_at,
       p.key AS project_key
  FROM task t
  JOIN project p ON p.id = t.project_id
 WHERE t.id = :'probe_task'
   AND t.workspace_id = :'ws_id'
   AND t.deleted_at IS NULL
   AND p.deleted_at IS NULL
   AND (   p.visibility = 'WORKSPACE'
        OR (p.visibility = 'TEAM' AND p.team_id = ANY (ARRAY[:'probe_team']::uuid[]))
        OR EXISTS (SELECT 1 FROM project_membership pm
                    WHERE pm.project_id = p.id AND pm.user_id = :'probe_user')
        OR p.id = ANY (:accessible_projects))
