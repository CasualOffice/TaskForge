-- name: Dead-letter depth by consumer (outbox_dlq_depth)
-- serves: docs/26 §Outbox & workers — outbox_delivery_dlq_ix
-- expects-index: outbox_delivery_dlq_ix
--
-- `dispatch::dlq_depth`, the gauge RB-02 pages on. Its cost grows forever and
-- never shrinks: dead-lettered rows are deliberately never swept (docs/25, "a
-- dead-lettered event is never silently dropped"), so this set is monotonic
-- until an operator drains it by hand.
--
-- Migration 0018 leads outbox_delivery_dlq_ix with `consumer` so the count is an
-- index-only scan feeding a grouped aggregate. Before that the index carried no
-- `consumer` column at all and the GROUP BY paid one random heap read per dead
-- row — the exact shape of a metric that gets slower the longer an incident goes
-- unresolved.
SELECT consumer, count(*)
  FROM outbox_delivery
 WHERE dead_lettered_at IS NOT NULL
 GROUP BY consumer
