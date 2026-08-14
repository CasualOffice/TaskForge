-- 0033 — Make attachment byte verification a claimable state transition.
--
-- Object-store HEAD/read calls cannot run inside the database transaction
-- (docs/10, docs/28). Two commit requests may therefore verify the same bytes
-- concurrently. `verified_at` lets the second transaction claim the database
-- transition exactly once after I/O; only the winner writes history and the
-- outbox event that wakes the scanner.

ALTER TABLE attachment ADD COLUMN verified_at timestamptz;

COMMENT ON COLUMN attachment.verified_at IS
    'When commit verified size and magic bytes. NULL until one request claims verification.';
