-- name: Outbox per-aggregate ordering (the claim's NOT EXISTS)
-- serves: docs/26 §Outbox & workers — outbox_event_aggregate_ix
-- expects-index: outbox_event_aggregate_ix
--
-- The correlated subquery inside `dispatch::claim`, with its outer row bound —
-- which is how the planner actually evaluates it: once per candidate delivery,
-- with `e.aggregate_id`, `e.created_at` and `e.id` already known.
--
-- Query 20 plans the claim as a whole and cannot catch this. Given the whole
-- statement the planner is free to answer the anti-join by materialising every
-- pending delivery for the consumer and hashing it — no sequential scan, so the
-- gate passes, and the cost is O(pending) per poll regardless of the LIMIT.
-- Bound to one aggregate, the only way to answer without an index on
-- `outbox_event(aggregate_id, ...)` is to scan outbox_event, and that is what
-- this file asserts cannot happen.
--
-- The row-wise `(created_at, id) <` is kept exactly as `claim` writes it. Split
-- into two predicates it would still be correct and would no longer be an index
-- bound, which is the difference the trailing `id` column in the index exists
-- for.
SELECT 1
  FROM outbox_delivery prior
  JOIN outbox_event pe ON pe.id = prior.event_id
 WHERE prior.consumer = 'webhook_delivery'
   AND pe.aggregate_id = :'probe_task'::uuid
   AND prior.dispatched_at IS NULL
   AND prior.dead_lettered_at IS NULL
   AND (pe.created_at, pe.id) < (:'anchor'::timestamptz, :'probe_outbox_event'::uuid)
 LIMIT 1
