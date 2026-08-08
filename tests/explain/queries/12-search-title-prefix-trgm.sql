-- name: Command-palette title match (trigram)
-- serves: docs/26 §task_search — task_search_trgm (title_trgm gin_trgm_ops)
-- expects-index: task_search_trgm
--
-- The typeahead path: substring and typo tolerance, which tsvector cannot do.
-- Asserted separately from 11 because it uses a different GIN opclass, and an
-- opclass that is present but unusable by the emitted operator is a silent scan.
--
-- Same measured problem as 11: ILIKE resolves to texticlike, also not LEAKPROOF,
-- so under RLS the trigram index cannot be an index qual and the plan falls back
-- to the scope index plus a filter over the whole tenant.
SELECT s.task_id, s.title_trgm
  FROM task_search s
 WHERE s.workspace_id = :'ws_id'
   AND s.project_id = ANY (:accessible_projects)
   AND s.title_trgm ILIKE ('%' || :'probe_title_prefix' || '%')
 ORDER BY s.updated_at DESC
 LIMIT 51
