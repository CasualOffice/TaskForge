-- 0022 — The project an outbox event belongs to (C-015, docs/05 §Live updates).
--
-- WHY A CONSUMER CANNOT AUTHORIZE WITHOUT THIS
--
-- An SSE subscriber must never receive an event for a task they could not read
-- through `GET`. Readability in this product is decided at the **project**:
-- `project.VISIBLE` in casual-task-persistence resolves WORKSPACE / TEAM /
-- membership / project-scoped grant, and every task read is filtered through it.
--
-- `outbox_event` carried `workspace_id` and `aggregate_id` and nothing between
-- them. A fan-out consumer holding one of those rows therefore could not answer
-- "which project is this?" without reading the aggregate back — and for
-- `task.deleted` the aggregate is exactly what no longer reliably resolves. The
-- three ways out were:
--
--   1. Re-read the aggregate per event. A database round trip per event per
--      consumer, on the path whose whole purpose is to be fast, and wrong for
--      deletes.
--   2. Read it out of `payload`. The payload is handler-supplied JSON with no
--      schema the database enforces; a handler that omitted the field would
--      produce an event that silently authorizes against nothing. That is the
--      widest-blast-radius leak in the product, guarded by a convention.
--   3. Record it in the producing transaction, where it is already known.
--
-- This is 3. `Change.project_id` already exists and is already written to
-- `activity_event`; it was simply dropped on the floor for the outbox.
--
-- NULLABLE, AND WHAT NULL MEANS
--
-- Workspace-level events — a workspace renamed, a member removed — genuinely
-- have no project. NULL is that, and it is **not** a wildcard: a subscriber is
-- attached to one project, and an event that cannot prove it belongs to that
-- project is not delivered. Fail-closed is the only safe default for a filter
-- whose failure mode is every workspace seeing every event.
--
-- Existing rows are NULL, which under that rule means "not delivered to any
-- project stream". That is correct for a backfill: those events pre-date the
-- feature and no client is waiting for them.

ALTER TABLE outbox_event ADD COLUMN project_id uuid;

-- No index. The dispatcher claims by (consumer, next_attempt_at) and reads this
-- column from the row it already has; nothing queries `outbox_event` BY project.
-- Adding an index "because it is a foreign key" would be a write cost on the
-- hottest insert path in the system, paid for a read that does not exist.
--
-- No REFERENCES project(id) either, and that one is deliberate rather than
-- lazy: `docs/25` makes the outbox an immutable record of what happened, and a
-- foreign key would make a project delete either fail or cascade into that
-- record. History does not get to be edited by a later delete.

COMMENT ON COLUMN outbox_event.project_id IS
    'Authorization scope for fan-out (C-015). NULL = workspace-level event, '
    'never delivered to a project-scoped subscriber.';
