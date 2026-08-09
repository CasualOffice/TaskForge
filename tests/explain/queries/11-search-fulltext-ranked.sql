-- name: Full-text search with the permission pre-filter
-- serves: docs/26 §Permission filtering; task_search_gin + task_search_scope_ix
-- expects-index: task_search_gin
--
-- Filter BEFORE ranking. Searching first and filtering by permission afterwards
-- collapses page sizes, breaks cursors, and makes result counts lie (docs/26).
-- `s.project_id = ANY(...)` is the actor's accessible project set, resolved once
-- per request and cached by authz_epoch (docs/04).
--
-- LIMIT 51 rather than 50: the extra row is how "has next page" is detected
-- without a count.
--
-- KNOWN, MEASURED, AND NOT FIXED HERE: under RLS this query does not use
-- task_search_gin at all. `tsvector @@ tsquery` resolves to ts_match_vq, which
-- is not marked LEAKPROOF, so PostgreSQL refuses to evaluate it before the row
-- security qual and therefore cannot use it as an index qual. The plan degrades
-- to "read every row of the tenant through task_search_scope_ix, then filter"
-- — 591 heap blocks instead of 2 on a 10k-task workspace, and linear in tenant
-- size thereafter. Disabling RLS on task_search alone flips it back to
-- BitmapAnd(task_search_gin, task_search_scope_ix).
--
-- It is not a Seq Scan, so this gate passes it. That is the honest limit of a
-- no-scan rule, and the expects-index advisory below is what makes the drift
-- visible until the question is settled (it touches ADR-014 and ADR-020, so it
-- is an ADR decision, not an implementation detail).
--
-- UPDATE — MEASURED AT REFERENCE SCALE, AND IT BECOMES A SEQ SCAN.
--
-- The sentence above ("it is not a Seq Scan") is true of THIS corpus and false
-- of the one the product targets. Measured on a loaded 2,000,000-task corpus
-- (tools/casual-task-seed --scale reference), same query, same 6%-selective
-- term, same instance:
--
--   as taskforge_app, RLS applied   ->  Parallel Seq Scan on task_search
--   as the owner, RLS not applied   ->  Bitmap Index Scan on task_search_gin
--
-- RLS is the only difference between those two plans. So the degradation this
-- comment describes is not a fixed cost: it worsens with tenant size, and
-- somewhere between 109k and 2M rows it stops being "read the tenant through
-- task_search_scope_ix" and becomes exactly the sequential scan on a
-- tenant-scale table that docs/26 NFR-5 and ADR-011 forbid.
--
-- The consequence for this gate, stated plainly: a green run here does NOT mean
-- the rule holds at reference scale. Plan choice depends on selectivity and on
-- table size, and this suite runs two orders of magnitude below the corpus the
-- rule is written about. Tracked as D-043.
--
-- C-013 UPDATE: this is now the query the compiler actually emits, character
-- for character in shape — the repository projection, the `CROSS JOIN
-- plainto_tsquery(...) q` that builds the tsquery once instead of twice, and
-- the ranking expression repeated in ORDER BY rather than the `rank` alias
-- (an alias is not visible in WHERE, so the keyset resume must use the
-- expression). Nothing above changes: the tenant predicate is already on
-- task_search itself, which is the "tenant-filtered projection" D-043 accepts
-- as the thing to try first, and it is not sufficient.
SELECT t.id, t.workspace_id, t.project_id, t.number, t.title, t.description,
       t.type::text AS "type", t.priority::text AS priority, t.status_id,
       t.state::text AS state,
       t.reporter_id, t.environment_id, t.milestone_id, t.parent_id,
       t.start_at, t.due_at, t.position, t.created_at, t.created_by,
       t.updated_at, t.updated_by, t.version, t.archived_at,
       ts_rank_cd(s.document, q) AS rank
  FROM task_search s
  JOIN task t ON t.id = s.task_id
  CROSS JOIN plainto_tsquery('english', :'probe_term') q
 WHERE s.workspace_id = :'ws_id'
   AND s.project_id = ANY (:accessible_projects)
   AND s.document @@ q
   AND t.workspace_id = :'ws_id'
   AND t.deleted_at IS NULL
   AND (TRUE)
 ORDER BY ts_rank_cd(s.document, q) DESC, t.id DESC
 LIMIT 51
