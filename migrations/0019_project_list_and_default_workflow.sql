-- 0019 — Two indexes C-006 needs. See docs/26-SEARCH-INDEXING-AND-QUERY.md.
--
-- Neither adds a column and neither changes a contract; both close a gap that
-- would otherwise be handled by hoping.

-- One default workflow per workspace.
--
-- docs/23: a project has exactly one workflow and the default "works with zero
-- configuration", but nothing creates one — so the first project create in a
-- workspace materializes it (casual-task-persistence::workflow). Two concurrent
-- first creates would otherwise each insert one, and the workspace would end up
-- with two workflows both claiming to be the default, with no error anywhere.
--
-- A check-then-insert cannot prevent that; a unique index can. Partial, because
-- non-default workflows are unlimited by design (docs/23: "workflows are
-- workspace-level objects and may be shared by many projects").
CREATE UNIQUE INDEX workflow_default_uq ON workflow (workspace_id) WHERE is_default;

-- The project list's cursor.
--
-- GET /api/v1/projects is keyset-paginated on (created_at DESC, id DESC)
-- (docs/05 §Pagination, docs/26 §Cursor pagination). project_ws_ix
-- (workspace_id, archived_at) serves the tenant filter but supplies no order,
-- so every page would sort. Same shape as task_list_ix, one level up.
--
-- `project` is not a tenant-scale table — docs/26's reference corpus is 200
-- projects per workspace — so this is not what the no-seq-scan gate is for. It
-- is here because AGENTS.md admits no query path without its index, and a
-- sortable field acquires its index in the PR that makes it sortable.
CREATE INDEX project_list_ix
    ON project (workspace_id, created_at DESC, id DESC) WHERE deleted_at IS NULL;
