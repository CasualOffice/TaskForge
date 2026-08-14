-- 0034 — The two bounded outbox-retention scans (C-011, docs/25).
--
-- The dispatch table is intentionally mostly completed history: the pending
-- index stays small by excluding delivered rows, which means it cannot serve
-- the seven-day cleanup. Without a separate index the hourly worker scans the
-- whole table to remove its oldest 1,000 rows.
--
-- Oldest first is part of the bound. It gives every run deterministic progress
-- through a backlog rather than repeatedly finding arbitrary eligible pages.
CREATE INDEX outbox_delivery_retention_ix
    ON outbox_delivery (dispatched_at, id)
    WHERE dispatched_at IS NOT NULL;

-- The event is removed only after its last delivery is gone. `created_at`
-- finds the oldest candidate batch; the UNIQUE(event_id, consumer) index from
-- migration 0013 answers the correlated existence check.
CREATE INDEX outbox_event_retention_ix
    ON outbox_event (created_at, id);
