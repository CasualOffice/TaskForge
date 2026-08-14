-- name: Outbox delivered-row retention batch
-- serves: docs/26 §Outbox & workers — outbox_delivery_retention_ix
-- expects-index: outbox_delivery_retention_ix
--
-- The candidate half of `dispatch::sweep`. The enclosing DELETE addresses the
-- selected primary keys and does not change how PostgreSQL finds this bounded
-- oldest-first batch. The table is mostly delivered history in this corpus, so
-- this proves the cleanup path has its own index instead of borrowing the tiny
-- pending index.
SELECT id
  FROM outbox_delivery
 WHERE dispatched_at IS NOT NULL
   AND dispatched_at < now() - interval '7 days'
 ORDER BY dispatched_at, id
 LIMIT 1000
