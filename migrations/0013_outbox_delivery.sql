-- 0013 — Per-consumer outbox delivery state (D-038, docs/25 §Dispatch).
--
-- WHY THIS TABLE EXISTS RATHER THAN MORE COLUMNS ON outbox_event
--
-- docs/25 specifies six consumers "each independently retried and independently
-- failing". `outbox_event.dispatched_at`, `attempts` and `last_error` are one
-- set of values for the whole event, so a webhook failing while the search
-- projection succeeds has nowhere to record either outcome — the two would
-- overwrite each other, and a retry for one would re-deliver to all six.
--
-- Delivery state therefore belongs to (event, consumer). The event row stays
-- what it always was: the immutable fact, written in the producing transaction.
--
-- WHY next_attempt_at IS A COLUMN AND NOT A SLEEP
--
-- docs/25's backoff ladder — 1 s, 4 s, 16 s, 1 m, 5 m, 30 m — was previously
-- undeliverable: nothing recorded WHEN to retry, so the claim query had no way
-- to exclude a row that was waiting, and a backoff held in a worker's memory is
-- lost the moment it restarts.

CREATE TABLE outbox_delivery (
    id               uuid PRIMARY KEY,
    workspace_id     uuid NOT NULL,
    event_id         uuid NOT NULL REFERENCES outbox_event(id) ON DELETE CASCADE,

    -- Which of the six. Text rather than an enum: docs/34 lets a plugin
    -- subscribe, so the set is open at runtime in a way an enum would fight.
    consumer         text NOT NULL,

    attempts         integer     NOT NULL DEFAULT 0,
    -- Due immediately on creation; pushed forward by the backoff ladder.
    next_attempt_at  timestamptz NOT NULL DEFAULT now(),

    -- Set at claim, cleared on record. A row claimed longer than the expiry in
    -- docs/25 is reclaimable: a worker that dies between claim and record would
    -- otherwise leave it undelivered forever.
    claimed_at       timestamptz,
    claimed_by       text,

    dispatched_at    timestamptz,
    dead_lettered_at timestamptz,
    last_error       text,

    created_at       timestamptz NOT NULL DEFAULT now(),

    -- One delivery per consumer per event. This is what makes the producing
    -- transaction's fan-out insert idempotent, and what stops a replay from
    -- creating a second delivery for a consumer that already had one.
    UNIQUE (event_id, consumer)
);

-- The claim query's index.
--
-- Led by `consumer` because a dispatcher always polls for exactly one: an index
-- led by `next_attempt_at` makes a worker walk five other consumers' due rows
-- to find its own, and gets worse as consumers are added. The planner said so
-- before this comment did — it preferred a different index over the
-- time-leading version of this one, which is what prompted the change.
--
-- Partial, so it holds only work that is actually outstanding. Dispatched rows
-- leave it, which keeps it small on a table that grows forever; dead-lettered
-- rows leave it too, and that one is load-bearing — dead rows are by definition
-- the OLDEST pending rows, so without that predicate a growing dead-letter
-- queue would sit permanently at the head of the index and be re-read on every
-- poll.
CREATE INDEX outbox_delivery_pending_ix
    ON outbox_delivery (consumer, next_attempt_at, created_at)
    WHERE dispatched_at IS NULL AND dead_lettered_at IS NULL;

-- docs/46 alerts on DLQ depth, and a count over a partial index is cheap.
CREATE INDEX outbox_delivery_dlq_ix
    ON outbox_delivery (workspace_id, dead_lettered_at)
    WHERE dead_lettered_at IS NOT NULL;

-- Per-aggregate ordering (docs/25 §Delivery semantics) needs to ask "is there
-- an earlier undelivered event for this aggregate?" cheaply.
CREATE INDEX outbox_delivery_ordering_ix
    ON outbox_delivery (consumer, event_id)
    WHERE dispatched_at IS NULL AND dead_lettered_at IS NULL;

-- Tenant isolation, matching migration 0010's policy exactly. This table is
-- created after 0010's catalogue loop ran, so it does not get a policy for
-- free — and a tenant table without one is the silent failure that whole
-- mechanism exists to prevent.
ALTER TABLE outbox_delivery ENABLE ROW LEVEL SECURITY;
ALTER TABLE outbox_delivery FORCE ROW LEVEL SECURITY;
CREATE POLICY outbox_delivery_tenant_isolation ON outbox_delivery
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

-- The dispatcher polls across all tenants, exactly as outbox_event does
-- (migration 0010 exempts that table for the same reason). A dispatcher that
-- had to know every workspace id in advance could not be a background worker.
-- The exemption is written here rather than assumed.
--
-- NOTE: the policy above still applies to the application role. The dispatcher
-- connects as a role that bypasses it, and that role does not exist yet — it
-- arrives with the worker in C-011's runtime half. Until then this table is
-- reachable only within a workspace scope, which is the safe direction.

GRANT SELECT, INSERT, UPDATE ON outbox_delivery TO taskforge_app;

-- THE OLD COLUMNS GO, RATHER THAN BEING LEFT ALONE
--
-- outbox_event.dispatched_at, .attempts and .last_error are now expressed
-- per-consumer above. Leaving them in place would be the more cautious-looking
-- choice and the more dangerous one: a dispatcher that updated
-- outbox_event.dispatched_at would run without error, report success, and
-- deliver nothing to five of the six consumers. Two places to record the same
-- fact is how they disagree.
--
-- Dropping them turns that mistake into a compile-time-equivalent failure — the
-- query errors on an unknown column the first time it runs, in a test, instead
-- of being discovered as missing notifications.
--
-- The partial indexes go with them; both are defined over dropped columns and
-- would fall anyway. They are named here so the loss is deliberate rather than
-- a cascade nobody read. Their replacements are outbox_delivery_pending_ix and
-- outbox_delivery_dlq_ix, and RB-02 in docs/50 is rewritten against those.

DROP INDEX outbox_pending_ix;
DROP INDEX outbox_dlq_ix;

ALTER TABLE outbox_event
    DROP COLUMN dispatched_at,
    DROP COLUMN attempts,
    DROP COLUMN last_error;
