-- name: Authorization resolver — grants for a principal set
-- serves: docs/26 §Authorization tables — role_assignment_lookup_ix; role_permission_ix
-- expects-index: role_assignment_lookup_ix
--
-- The hot path: read on EVERY request, before the request's own query runs
-- (docs/04). principals = {actor} ∪ teams_of(actor), so the lookup is over a
-- principal SET, not a single principal.
--
-- Written as a UNION ALL of two single-principal-type probes rather than one
-- OR'd predicate. An OR across (principal_type, principal_id) forces the planner
-- to choose between a BitmapOr and a scan, and the scan wins as soon as the
-- estimate slips; UNION ALL cannot degrade that way, and the two arms are the
-- same index.
SELECT ra.role_id, ra.scope_type, ra.scope_id, ra.constraints, rp.permission
  FROM role_assignment ra
  JOIN role_permission rp ON rp.role_id = ra.role_id
 WHERE ra.workspace_id = :'ws_id'
   AND ra.principal_type = 'USER'::principal_type
   AND ra.principal_id = :'probe_user'
UNION ALL
SELECT ra.role_id, ra.scope_type, ra.scope_id, ra.constraints, rp.permission
  FROM role_assignment ra
  JOIN role_permission rp ON rp.role_id = ra.role_id
 WHERE ra.workspace_id = :'ws_id'
   AND ra.principal_type = 'TEAM'::principal_type
   AND ra.principal_id = ANY (ARRAY[:'probe_team']::uuid[])
