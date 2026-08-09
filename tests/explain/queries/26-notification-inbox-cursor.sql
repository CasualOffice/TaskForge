-- name: Notification inbox, unread first, second page (keyset cursor)
-- serves: docs/29 §The inbox; notification_inbox_ix (migration 0022)
-- expects-index: notification_inbox_ix
--
-- GET /api/v1/notifications. docs/29 orders the inbox "with unread first", which
-- makes the unread flag the LEADING sort key — and that is what
-- notification_unread_ix (migration 0008) cannot serve: it is partial on
-- `read_at IS NULL`, so it holds only half the page. It serves the badge, which
-- is case 19.
--
-- The expression index is the whole point. `(read_at IS NULL)` is IMMUTABLE and
-- therefore indexable, so ordering by it costs no sort. Without it this is a
-- sort of the recipient's entire mailbox on every page, on a tenant-scale table
-- (tests/explain/tenant-scale-tables.txt lists `notification`).
--
-- The cursor is a row comparison across all three keys, which works as a single
-- `<` because all three sort DESC. The id tiebreaker is mandatory (docs/26):
-- two notifications written in the same millisecond would otherwise make a page
-- repeat or skip a row, and the fan-out writes in batches.
SELECT n.id, n.user_id, n.event_type, n.reason, n.aggregate_id, n.payload,
       n.created_at, n.read_at
  FROM notification n
 WHERE n.workspace_id = :'ws_id'
   AND n.user_id = :'probe_user'
   AND ((n.read_at IS NULL), n.created_at, n.id)
       < (true, :'cursor_updated_at'::timestamptz, :'cursor_id'::uuid)
 ORDER BY (n.read_at IS NULL) DESC, n.created_at DESC, n.id DESC
 LIMIT 51
