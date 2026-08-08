-- name: Task activity stream (the history tab)
-- serves: docs/26 §activity_event — activity_stream_ix
--         (workspace_id, aggregate_id, occurred_at DESC)
--
-- activity_event is range-partitioned monthly (ADR-021), so the plan names a
-- partition, never the parent. The assertion resolves partitions through
-- pg_inherits for exactly this reason.
SELECT a.id, a.event_type, a.actor_id, a.changes, a.occurred_at
  FROM activity_event a
 WHERE a.workspace_id = :'ws_id'
   AND a.aggregate_id = :'probe_task'
 ORDER BY a.occurred_at DESC
 LIMIT 51
