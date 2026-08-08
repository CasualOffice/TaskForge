-- name: Outbox dispatch poll
-- serves: docs/26 §Outbox & workers — outbox_delivery_pending_ix partial
-- expects-index: outbox_delivery_pending_ix
--
-- The claim query from crates/casual-task-persistence/src/dispatch.rs, minus
-- FOR UPDATE SKIP LOCKED (EXPLAIN would have to lock rows to plan it) and with
-- the enclosing UPDATE dropped. The locking clause does not change the scan
-- node, which is what this gate asserts.
--
-- The NOT EXISTS is kept, deliberately. It is what enforces per-aggregate
-- ordering (docs/25 §Delivery semantics), it runs once per candidate row, and a
-- correlated subquery is exactly the kind of thing that quietly turns a poll
-- into a sequential scan. A probe that dropped it would gate a query nobody
-- runs.
--
-- outbox_event is exempt from RLS (migration 0010) because the dispatcher polls
-- across all tenants. outbox_delivery is NOT exempt: it carries workspace_id
-- and has its own policy in migration 0013. The dispatcher will reach it as a
-- role that bypasses RLS, which is why this probe plans without a tenant
-- predicate.
SELECT c.id
  FROM outbox_delivery c
  JOIN outbox_event e ON e.id = c.event_id
 WHERE c.consumer = 'webhook_delivery'
   AND c.dispatched_at IS NULL
   AND c.dead_lettered_at IS NULL
   AND c.next_attempt_at <= now()
   AND (c.claimed_at IS NULL OR c.claimed_at < now() - interval '5 minutes')
   AND NOT EXISTS (
       SELECT 1
         FROM outbox_delivery prior
         JOIN outbox_event pe ON pe.id = prior.event_id
        WHERE prior.consumer = c.consumer
          AND pe.aggregate_id = e.aggregate_id
          AND prior.dispatched_at IS NULL
          AND prior.dead_lettered_at IS NULL
          AND (pe.created_at, pe.id) < (e.created_at, e.id))
 ORDER BY e.created_at, e.id
 LIMIT 100
