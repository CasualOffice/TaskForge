-- 0024 — What the notification fan-out needs from the outbox, and the index the
-- inbox is served by (C-016). See docs/29, docs/25 §Event envelope.

-- ---------------------------------------------------------------------------
-- 1. THE ACTOR, ON THE EVENT
-- ---------------------------------------------------------------------------
--
-- docs/29 §Batching and suppression, rule 1: "Self-action suppression. You are
-- never notified about your own action. Obvious, and omitted often enough to be
-- worth stating."
--
-- It was not expressible. `outbox_event` carried no actor, so a consumer
-- reading an event could not tell who caused it — and the one rule every
-- tracker is complained about for getting wrong is exactly the one the schema
-- made impossible to implement. The alternatives were worse: joining
-- `activity_event` on (aggregate_id, event_type, occurred_at) to recover the
-- actor is a guess dressed as a join, and requiring every producer to repeat
-- the actor inside `payload` is a convention, which survives until the first
-- author who does not know about it.
--
-- docs/25 §Event envelope already specifies `"actor": {"type", "id",
-- "display_name"}` as part of every event. This is that field, stored rather
-- than described.
--
-- NULLABLE, because `Provenance.actor` is `Option<UserId>`: docs/25 records a
-- system-generated change with no actor, and a retention sweep or an automation
-- running as nobody is a real event with a real absence rather than a missing
-- value. A NULL actor suppresses nothing, which is correct — nobody's own
-- action caused it.
ALTER TABLE outbox_event ADD COLUMN actor_id uuid;

COMMENT ON COLUMN outbox_event.actor_id IS
    'Who caused this event; NULL for system-generated. Read by the notification '
    'fan-out for docs/29 self-action suppression, which cannot be implemented '
    'without it.';

-- ---------------------------------------------------------------------------
-- 2. THE INBOX INDEX
-- ---------------------------------------------------------------------------
--
-- docs/29 §The inbox: "cursor-paginated ... Grouped by task, with unread
-- first."
--
-- `notification_unread_ix` (migration 0008) is partial on `read_at IS NULL`, so
-- it serves the unread *badge* — an index-only count — and cannot serve a page
-- that contains read rows too. Ordering unread-before-read across the whole
-- inbox needs the unread flag to BE the leading sort key, and an expression
-- index is what makes that index-served rather than a sort of the entire
-- mailbox.
--
-- `(read_at IS NULL)` is IMMUTABLE, so it is indexable. DESC on it puts true —
-- unread — first, which is the documented order. `id` is the mandatory cursor
-- tiebreaker (docs/26): without it, two notifications written in the same
-- millisecond make a page repeat or skip a row.
--
-- `notification` is a tenant-scale table (tests/explain/tenant-scale-tables.txt),
-- so this is not decoration: a sequential scan here fails the
-- explain-no-seq-scan gate, and tests/explain/queries/24 asserts the plan.
CREATE INDEX notification_inbox_ix
    ON notification (user_id, (read_at IS NULL) DESC, created_at DESC, id DESC);

-- Coalescing (docs/29 §Batching and suppression, rule 2) looks for an existing
-- unread notification about the same aggregate for the same recipient. Without
-- this it is a scan of the user's whole unread set on every delivered event —
-- and the fan-out runs on every event in the system.
CREATE INDEX notification_coalesce_ix
    ON notification (user_id, aggregate_id, created_at DESC) WHERE read_at IS NULL;
