-- name: Outbox orphan-event retention batch
-- serves: docs/26 §Outbox & workers — outbox_event_retention_ix
-- expects-index: outbox_event_retention_ix
--
-- The candidate half of `dispatch::sweep`. The NOT EXISTS is load-bearing: an
-- old event stays until every consumer's delivery is gone. The event index
-- supplies the bounded oldest-first walk and the existing
-- outbox_delivery_event_id_consumer_key answers the correlated lookup.
SELECT e.id
  FROM outbox_event e
 WHERE e.created_at < now() - interval '7 days'
   AND NOT EXISTS (
       SELECT 1 FROM outbox_delivery d WHERE d.event_id = e.id)
 ORDER BY e.created_at, e.id
 LIMIT 1000
