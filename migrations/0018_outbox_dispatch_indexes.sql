-- 0018 — The two indexes the dispatch path asks for and did not have.
--
-- Migration 0013 gave outbox_delivery three indexes and left outbox_event with
-- nothing but its primary key: 0007's two partial indexes were defined over
-- columns 0013 dropped, so they fell with them. Nothing replaced them, because
-- at the time the claim query was read as "a scan of outbox_delivery" and the
-- event table looked like a lookup by id.
--
-- It is not. Two of the dispatch path's reads address outbox_event by something
-- other than its primary key, and both are in the poll loop.

-- WHY: the per-aggregate ordering anti-join (docs/25 §Delivery semantics).
--
-- crates/casual-task-persistence/src/dispatch.rs, `claim`, asks per candidate
-- row: "is there an EARLIER undelivered event for this aggregate?"
--
--     NOT EXISTS (SELECT 1 FROM outbox_delivery prior
--                   JOIN outbox_event pe ON pe.id = prior.event_id
--                  WHERE pe.aggregate_id = e.aggregate_id
--                    AND (pe.created_at, pe.id) < (e.created_at, e.id) ...)
--
-- With no index on `aggregate_id`, that predicate has exactly two plans and both
-- are wrong at scale: a sequential scan of outbox_event per candidate, or — the
-- one the planner actually chose — a hash anti-join that first materialises
-- EVERY pending delivery for the consumer joined to EVERY one of its events, to
-- answer a question about at most a handful of rows. The LIMIT bounds what is
-- claimed; it does not bound that. At a backlog of two million pending
-- deliveries the poll hashes two million rows to claim sixty-four, in the
-- transaction that holds the claim's row locks.
--
-- Not partial: undeliverability is a property of outbox_delivery, not of the
-- event, so there is no predicate here that would keep the index small. It is
-- kept small by the 7-day sweep instead (`dispatch::sweep`).
--
-- The trailing `id` is load-bearing rather than decorative: the comparison is
-- the row-wise `(created_at, id) < (created_at, id)`, which is what makes the
-- ordering total across events created in the same instant, and PostgreSQL can
-- use a row comparison as an index bound only when the index carries the same
-- columns in the same order.
CREATE INDEX outbox_event_aggregate_ix ON outbox_event (aggregate_id, created_at, id);

-- WHY: outbox_dlq_depth is a count grouped by consumer, and `consumer` was not
-- in the index it reads.
--
-- The old definition (0013) led with workspace_id and did not carry `consumer`
-- at all, so the gauge's GROUP BY had to fetch a heap tuple for every dead row
-- to learn which consumer it belonged to, then sort. Dead letters are NEVER
-- swept — docs/25: "a dead-lettered event is never silently dropped" — so that
-- set only grows, and a metric that costs one random heap read per dead row is a
-- metric that gets slower every week it is not at zero.
--
-- Led by `consumer` so the gauge is an index-only scan feeding a grouped
-- aggregate with no heap access and no sort. RB-02's second query groups by
-- workspace_id instead; it reads the whole partial index either way, so leading
-- with consumer costs it nothing.
--
-- The cost, stated: the count is still O(dead letters). Nothing short of a
-- maintained counter changes that, and a counter that can disagree with the
-- table is a worse thing to page an operator with than a scan that cannot.
DROP INDEX outbox_delivery_dlq_ix;
CREATE INDEX outbox_delivery_dlq_ix
    ON outbox_delivery (consumer, workspace_id, dead_lettered_at)
    WHERE dead_lettered_at IS NOT NULL;
