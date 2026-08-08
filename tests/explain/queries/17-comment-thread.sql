-- name: Comment thread of a task
-- serves: docs/26 §Everything else — comment_task_ix (task_id, created_at)
-- expects-index: comment_task_ix
SELECT c.id, c.author_id, c.body, c.created_at, c.parent_comment_id
  FROM comment c
 WHERE c.task_id = :'probe_task'
   AND c.deleted_at IS NULL
 ORDER BY c.created_at ASC
 LIMIT 51
