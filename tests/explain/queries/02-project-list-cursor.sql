-- name: Project list, second page (keyset cursor)
-- serves: docs/26 §task — task_list_ix (project_id, updated_at DESC, id DESC)
-- expects-index: task_list_ix
--
-- The cursor is (sort key, id) as a row comparison, exactly as docs/26 §Cursor
-- pagination specifies. The id tiebreaker is mandatory: without it, ties in
-- updated_at make the cursor non-deterministic under concurrent writes.
SELECT t.id, t.number, t.title, t.updated_at, t.state, t.priority
  FROM task t
 WHERE t.project_id = :'probe_project'
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
   AND (t.updated_at, t.id) < (:'cursor_updated_at'::timestamptz, :'cursor_id'::uuid)
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
