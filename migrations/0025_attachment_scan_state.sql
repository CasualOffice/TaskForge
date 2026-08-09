-- 0025 — Attachment scan state and the thread's cursor (C-010).
--
-- Migration 0006 already created `attachment` with `scan_status`,
-- `committed_at`, and the partial index that makes an uncommitted row invisible
-- (docs/28 §The invariant). This adds the three things the pipeline needs that
-- it does not have.

-- ---------------------------------------------------------------------------
-- 1. WHEN the scan resolved, and WHY it resolved that way
-- ---------------------------------------------------------------------------
--
-- `scan_status` records the verdict; nothing records when it was reached or
-- what it said. Both are needed by the two readers docs/28 names:
--
--   * The orphan sweeper distinguishes "PENDING since a minute ago" from
--     "PENDING since last Tuesday" — the first is in flight, the second is a
--     scan worker that died. Without a timestamp, the sweeper's only options
--     are to re-scan everything or nothing.
--   * Dead-letter review (RB-02's shape, one table over) needs the scanner's
--     message. "INFECTED" alone cannot be triaged; "INFECTED: Eicar-Test-
--     Signature" can.
ALTER TABLE attachment ADD COLUMN scanned_at  timestamptz;
ALTER TABLE attachment ADD COLUMN scan_detail text;

-- ---------------------------------------------------------------------------
-- 2. The files-tab cursor
-- ---------------------------------------------------------------------------
--
-- `attachment_task_ix` is `(task_id)` with the visibility predicate — it finds
-- a task's files but imposes no order, so every page of the files tab would
-- sort. This is the same shape as `task_list_ix` and `project_list_ix` one
-- level down: the sort key and the id tiebreaker, under the same partial
-- predicate so an uncommitted row is not in the index a read uses (docs/26,
-- docs/28).
--
-- It does not replace `attachment_task_ix`: that one stays the cheaper probe
-- for "does this task have files at all".
CREATE INDEX attachment_thread_ix ON attachment (task_id, created_at DESC, id DESC)
    WHERE committed_at IS NOT NULL AND deleted_at IS NULL;

-- ---------------------------------------------------------------------------
-- 3. The scan queue's own index
-- ---------------------------------------------------------------------------
--
-- The scan worker asks "what is committed-pending, oldest first". Without this
-- it reads `attachment_pending_ix` — which is `(created_at) WHERE committed_at
-- IS NULL`, the complement — and finds every row that was pre-signed and never
-- uploaded mixed in with the ones actually waiting on a verdict. Those are
-- different questions and, after a burst of abandoned uploads, wildly different
-- row counts.
CREATE INDEX attachment_scan_queue_ix ON attachment (created_at)
    WHERE scan_status = 'PENDING' AND deleted_at IS NULL;

COMMENT ON COLUMN attachment.scanned_at IS
    'When scan_status last changed. NULL while PENDING and never scanned.';
COMMENT ON COLUMN attachment.scan_detail IS
    'The scanner''s message for a non-CLEAN verdict. Never shown to a client — '
    'it names detection signatures, which is reconnaissance.';
