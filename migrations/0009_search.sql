-- 0009 — The search projection. See docs/26-SEARCH-INDEXING-AND-QUERY.md.
--
-- Full-text does NOT run against `task` directly. A separate projection table
-- because:
--   1. `task` is the hot write path, and GIN maintenance is bursty — the exact
--      latency spike you do not want on a drag-and-drop board.
--   2. The document spans more than the task row (tags, assignee names, comment
--      bodies); a generated column cannot see other tables.
--   3. It is the seam an external engine would replace (ADR-014).

CREATE TABLE task_search (
    task_id       uuid PRIMARY KEY REFERENCES task(id) ON DELETE CASCADE,
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    document      tsvector NOT NULL,
    title_trgm    text NOT NULL,
    updated_at    timestamptz NOT NULL
);

CREATE INDEX task_search_gin      ON task_search USING gin (document);
CREATE INDEX task_search_trgm     ON task_search USING gin (title_trgm gin_trgm_ops);
-- The permission pre-filter: project_id = ANY($accessible) (docs/26).
CREATE INDEX task_search_scope_ix ON task_search (workspace_id, project_id);
