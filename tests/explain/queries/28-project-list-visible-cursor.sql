-- name: Project list with the visibility predicate, second page
-- serves: docs/26 §Everything else — project_ws_ix, project_list_ix (0018)
-- expects-index: project_list_ix
--
-- GET /api/v1/projects. The four-branch predicate is docs/04
-- §Visibility vs permission compiled to SQL, and it is joined onto the query
-- rather than applied afterwards — docs/04 §The list problem: post-filtering an
-- authorized page "silently shrinks pages and breaks cursors".
--
-- `project` is NOT a tenant-scale table (docs/26's reference corpus is 200
-- projects per workspace), so a scan here would not fail this gate. The probe
-- is here because the query exists and a plan nobody looks at is a plan nobody
-- notices changing.
SELECT p.id, p.workspace_id, p.key, p.name, p.description,
       p.visibility::text AS visibility, p.workflow_id, p.created_at, p.created_by,
       p.updated_at, p.updated_by, p.version, p.archived_at
  FROM project p
 WHERE p.workspace_id = :'ws_id'
   AND p.deleted_at IS NULL
   AND (   p.visibility = 'WORKSPACE'
        OR (p.visibility = 'TEAM'
            AND EXISTS (SELECT 1 FROM project_team pt
                         JOIN team_membership tm ON tm.team_id = pt.team_id
                        WHERE pt.project_id = p.id AND tm.user_id = :'probe_user'))
        OR EXISTS (SELECT 1 FROM project_membership pm
                    WHERE pm.project_id = p.id AND pm.user_id = :'probe_user')
        OR p.id = ANY (:accessible_projects))
   AND (p.created_at, p.id) < (:'cursor_updated_at'::timestamptz, :'cursor_id'::uuid)
 ORDER BY p.created_at DESC, p.id DESC
 LIMIT 51
