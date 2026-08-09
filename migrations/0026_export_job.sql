-- 0026 — Export jobs (C-021, docs/38 §Export is a job, not a request).
--
-- WHY A TABLE AND NOT A REQUEST
--
-- docs/38: "Anything above 1,000 rows is asynchronous." A synchronous export of
-- a large result set holds an HTTP connection and a database transaction for
-- minutes — a pinned connection per exporting user, a transaction long enough to
-- hold back vacuum, and a client whose only recovery from a timeout is to start
-- the whole thing again. The same reasoning D-038 applied to outbox dispatch.
--
-- So the request records intent and returns; a worker does the work. This table
-- is the intent, its progress, and where the artefact ended up.

CREATE TABLE export_job (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL,
    requested_by   uuid NOT NULL REFERENCES user_account(id),

    -- The filter as the client sent it: the LIST ENDPOINT'S OWN QUERY STRING.
    --
    -- Not SQL — SQL in a queue is a stored injection vector, and the worker must
    -- recompile anyway to inject the current permission predicate. Not a
    -- serialised AST either: the AST is an internal type with no wire form, and
    -- giving it one would be a second representation of the filter grammar to
    -- keep in step with the first. The query string is the representation that
    -- already exists, already has a parser, and is already what docs/38 means by
    -- "this view, as a file".
    filter_query   text NOT NULL,
    -- 'csv' | 'jsonl'. Text rather than an enum: docs/38 names XLSX as a third
    -- format arriving through OpenCalc, and an enum would need a migration to
    -- admit it.
    format         text NOT NULL,
    -- The chosen columns, in order. NULL means the default set.
    columns        jsonb,

    -- queued | running | succeeded | failed | expired
    status         text NOT NULL DEFAULT 'queued',
    -- Rows written so far. Updated per batch, so `GET /exports/{id}` can report
    -- progress rather than "still going".
    row_count      bigint NOT NULL DEFAULT 0,
    -- Where the artefact lives in object storage. NULL until the first byte is
    -- written.
    object_key     text,
    byte_size      bigint,
    -- Why it failed, for the requester. Never a database error verbatim.
    failure_reason text,

    -- Claim fields, the same shape as outbox_delivery (migration 0013): a
    -- worker that dies between claim and completion must not leave a job
    -- invisible forever.
    claimed_at     timestamptz,
    claimed_by     text,

    created_at     timestamptz NOT NULL DEFAULT now(),
    started_at     timestamptz,
    completed_at   timestamptz,
    -- docs/38: "Artifacts are deleted after 7 days." Stored rather than derived
    -- so a retention change does not silently reinterpret existing rows.
    expires_at     timestamptz NOT NULL DEFAULT now() + interval '7 days'
);

-- The worker's claim query: oldest queued job first, across all tenants.
--
-- Partial on `status = 'queued'` so it holds only outstanding work. The same
-- argument as outbox_delivery_pending_ix: finished jobs are the overwhelming
-- majority of the table within a week and must not be walked to find the one
-- job that needs running.
CREATE INDEX export_job_queued_ix ON export_job (created_at)
    WHERE status = 'queued';

-- "My exports", the only read path a user has.
CREATE INDEX export_job_requester_ix ON export_job (workspace_id, requested_by, created_at DESC);

-- The sweeper's path (docs/38: deleted after 7 days).
CREATE INDEX export_job_expiry_ix ON export_job (expires_at)
    WHERE object_key IS NOT NULL;

-- Tenant isolation, matching migration 0010's policy exactly. This table is
-- created long after 0010's catalogue loop ran, so it does not get a policy for
-- free — and an export row names a filter over customer data.
ALTER TABLE export_job ENABLE ROW LEVEL SECURITY;
ALTER TABLE export_job FORCE ROW LEVEL SECURITY;
CREATE POLICY export_job_tenant_isolation ON export_job
    USING (workspace_id = NULLIF(current_setting('taskforge.workspace_id', true), '')::uuid);

GRANT SELECT, INSERT, UPDATE ON export_job TO taskforge_app;

-- The worker claims across tenants, exactly as it does for outbox_delivery, so
-- it connects as the role that bypasses the policy above (migration 0014).
GRANT SELECT, UPDATE ON export_job TO taskforge_dispatcher;
