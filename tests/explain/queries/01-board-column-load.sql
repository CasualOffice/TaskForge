-- name: Board — load the visible columns of one project
-- serves: docs/26 §task — task_board_ix (project_id, status_id, position)
-- expects-index: task_board_ix
--
-- The single most-executed read in the product. Ordering comes from `position`,
-- the lexicographic rank string (ADR-013), so the index supplies column order
-- and card order together.
SELECT t.id, t.number, t.title, t.status_id, t.position, t.priority, t.type, t.due_at
  FROM task t
 WHERE t.project_id = :'probe_project'
   AND t.status_id = ANY (:board_statuses)
   AND t.deleted_at IS NULL
   AND t.archived_at IS NULL
 ORDER BY t.status_id, t.position
 LIMIT 200
