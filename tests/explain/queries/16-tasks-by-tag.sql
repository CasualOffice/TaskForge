-- name: Tasks of a tag (the reverse direction)
-- serves: docs/26 §task_tag — task_tag_rev_ix (tag_id, task_id)
-- expects-index: task_tag_rev_ix
--
-- "The reverse index is the classic omission. Without it, 'show everything
-- tagged security' scans." (docs/26). The composite primary key (task_id,
-- tag_id) serves tags-of-a-task and is useless here, which is precisely why this
-- query has its own assertion.
SELECT t.id, t.number, t.title, t.updated_at
  FROM task_tag tt
  JOIN task t ON t.id = tt.task_id
 WHERE tt.tag_id = :'probe_tag'
   AND t.deleted_at IS NULL
   AND t.project_id = ANY (:accessible_projects)
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51
