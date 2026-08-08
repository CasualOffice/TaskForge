-- name: Outbox lag gauge (docs/46's primary health signal)
-- serves: docs/26 §Outbox & workers — outbox_delivery_pending_ix, index-only
-- expects-index: outbox_delivery_pending_ix
--
-- `dispatch::oldest_pending_seconds`. It is read on a fixed cadence by every
-- dispatch loop, and an aggregate cannot stop early: whatever this costs, it
-- costs over the entire pending set.
--
-- The three columns it touches — consumer, next_attempt_at, created_at — are
-- exactly outbox_delivery_pending_ix's, and the two IS NULL predicates are
-- exactly that index's partial condition, so the whole gauge is one index-only
-- scan. It joined outbox_event for min(e.created_at) until 0018's PR; that made
-- it one random heap fetch per pending row, and this file is here so a
-- reintroduced join shows up as a plan change rather than as a slow poll.
--
-- now() rather than the corpus anchor, deliberately: `next_attempt_at <= now()`
-- is what makes the reading "actionable" (D-047), and the plan for a stable
-- function differs from the plan for a literal.
SELECT EXTRACT(EPOCH FROM (now() - min(d.created_at)))::float8
  FROM outbox_delivery d
 WHERE d.consumer = 'webhook_delivery'
   AND d.dispatched_at IS NULL
   AND d.dead_lettered_at IS NULL
   AND d.next_attempt_at <= now()
