-- name: Outbox dispatch poll
-- serves: docs/26 §Outbox & workers — outbox_pending_ix (created_at) partial
-- expects-index: outbox_pending_ix
--
-- Verbatim from docs/25 §Dispatch, minus FOR UPDATE SKIP LOCKED, which EXPLAIN
-- would have to lock rows to plan. The locking clause does not change the scan
-- node, which is what this gate asserts.
--
-- outbox_event is the ONE table exempt from RLS (migration 0010): the dispatcher
-- polls across all tenants, so a per-request tenant predicate would break
-- delivery. It is protected by never being reachable from a request path.
SELECT id, workspace_id, event_type, aggregate_type, aggregate_id, payload
  FROM outbox_event
 WHERE dispatched_at IS NULL
 ORDER BY created_at
 LIMIT 100
