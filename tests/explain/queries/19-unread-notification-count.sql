-- name: Unread notification count (the inbox badge)
-- serves: docs/26 §Outbox & workers — notification_unread_ix, partial on read_at
-- expects-index: notification_unread_ix
--
-- Rendered on every page load, so it is the query most likely to be executed
-- millions of times a day for a result of "3". The partial index means the count
-- touches only unread rows, and read rows leave the index entirely as they are
-- read — the same trick as the outbox's pending index.
SELECT count(*)
  FROM notification n
 WHERE n.user_id = :'probe_user'
   AND n.read_at IS NULL
