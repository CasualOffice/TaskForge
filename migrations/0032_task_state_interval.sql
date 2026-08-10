-- 0032 — How long work spent in each state.
--
-- `docs/38` §Where the numbers come from. Cycle time, lead time, time-in-state
-- and throughput all ask the same question — how long was this task in that
-- state, and when did it leave — and none of them can be answered from `task`,
-- which holds only where the work is *now*.
--
-- The alternative is to replay the event stream per request, which is exactly
-- the unbounded query `docs/38` exists to prevent: an aggregate over every
-- status change a workspace has ever made, run by everyone at 9am. So the
-- occupancy is materialised once, by the outbox worker, and read as a bounded
-- aggregate over an index.
--
-- **It is a cache, not a source of truth.** Every row here is derivable from
-- `audit_event`, which is append-only and carries the status id and state on
-- both sides of every transition. A projection that cannot be rebuilt is a
-- second source of truth that will disagree with the first; this one is
-- rebuilt, per task, by the consumer that maintains it — the same code path, so
-- the rebuild cannot rot while the incremental path is exercised.

CREATE TABLE task_state_interval (
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id    uuid NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    state         task_state NOT NULL,
    status_id     uuid NOT NULL,
    entered_at    timestamptz NOT NULL,
    -- NULL means the task is in this state now. Exactly one open row per task,
    -- which `tsi_open_ix` below makes a rule rather than a hope.
    exited_at     timestamptz,
    -- Generated, so no writer can disagree with the two columns it comes from.
    duration      interval GENERATED ALWAYS AS (exited_at - entered_at) STORED,
    PRIMARY KEY (task_id, entered_at)
);

-- The shape every measure reads: "in this workspace and project, which tasks
-- entered this state within this window". Leading with the tenant because every
-- query is tenanted and the planner should never consider a cross-tenant scan.
CREATE INDEX tsi_cycle_ix ON task_state_interval
    (workspace_id, project_id, state, entered_at);

-- One open interval per task, enforced rather than assumed. A second open row
-- would double-count a task in every "time in state" aggregate and there would
-- be nothing on screen to say so.
CREATE UNIQUE INDEX tsi_open_ix ON task_state_interval (task_id)
    WHERE exited_at IS NULL;

-- The rebuild reads one task's transitions out of the audit stream, and
-- `audit_event` is indexed by workspace and by event type — neither of which
-- narrows to a task. Without this the per-task rebuild scans a partition per
-- delivery, which is the projection making the thing it exists to prevent.
CREATE INDEX audit_target_ix ON audit_event (target_id, occurred_at)
    WHERE target_id IS NOT NULL;

-- ── Row-level security ──────────────────────────────────────────────────────
--
-- The backstop, not the fence: every read goes through a scoped transaction
-- already. `NULLIF(..., '')` is load-bearing for the reason 0010 gives — a
-- transaction-local `set_config` reverts to the empty string rather than
-- unsetting, and casting '' to uuid raises.
ALTER TABLE task_state_interval ENABLE ROW LEVEL SECURITY;
ALTER TABLE task_state_interval FORCE ROW LEVEL SECURITY;
CREATE POLICY task_state_interval_tenant_isolation ON task_state_interval
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

-- ── Grants ──────────────────────────────────────────────────────────────────
--
-- DELETE *is* granted here, unlike the custody tables in 0031, and the reason is
-- the difference between a record and a cache. A custody chain you can edit is
-- not a custody chain; a projection you cannot rewrite is a projection that can
-- never be repaired. The rebuild deletes a task's rows and derives them again.
GRANT SELECT, INSERT, UPDATE, DELETE ON task_state_interval TO taskforge_app;
