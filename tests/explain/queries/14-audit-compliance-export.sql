-- name: Audit export by event type (compliance / security review)
-- serves: docs/26 §audit_event — audit_ix (workspace_id, event_type, occurred_at DESC)
--
-- Not a UI query, but user-reachable through the compliance export endpoint and
-- therefore in scope for NFR-5. audit_event is the table most likely to be the
-- largest in the database.
SELECT a.id, a.event_type, a.actor_id, a.target_id, a.occurred_at
  FROM audit_event a
 WHERE a.workspace_id = :'ws_id'
   AND a.event_type = 'role.granted'
   AND a.occurred_at >= (:'anchor'::timestamptz - interval '90 days')
 ORDER BY a.occurred_at DESC
 LIMIT 51
