-- 0007 — The three streams. See docs/25-EVENTS-OUTBOX-AND-AUDIT.md.
--
-- The domain change, its activity record, its audit record, and its outbox
-- event commit in ONE transaction. There is no interleaving in which a change
-- exists without its history.

CREATE TABLE outbox_event (
    id             uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL,
    event_type     text NOT NULL,
    aggregate_type text NOT NULL,
    aggregate_id   uuid NOT NULL,
    payload        jsonb NOT NULL,
    schema_version integer NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    dispatched_at  timestamptz,
    attempts       integer NOT NULL DEFAULT 0,
    last_error     text
);
-- Stays tiny because dispatched rows leave the partial index.
CREATE INDEX outbox_pending_ix ON outbox_event (created_at) WHERE dispatched_at IS NULL;
CREATE INDEX outbox_dlq_ix     ON outbox_event (created_at) WHERE attempts >= 6;

-- Monthly range partitions (ADR-021): retention is DROP TABLE partition, not a
-- DELETE of tens of millions of rows that bloats and vacuums for hours.
CREATE TABLE activity_event (
    id             uuid NOT NULL,
    workspace_id   uuid NOT NULL,
    project_id     uuid,
    aggregate_type text NOT NULL,
    aggregate_id   uuid NOT NULL,
    event_type     text NOT NULL,
    actor_id       uuid REFERENCES user_account(id),   -- NULL = system
    -- Holds display VALUES, not IDs: the stream is rendered years later,
    -- possibly after a status was renamed or deleted, and must still read
    -- correctly (docs/25).
    changes        jsonb NOT NULL DEFAULT '{}',
    occurred_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (id, occurred_at)
) PARTITION BY RANGE (occurred_at);

CREATE INDEX activity_stream_ix  ON activity_event (workspace_id, aggregate_id, occurred_at DESC);
CREATE INDEX activity_project_ix ON activity_event (project_id, occurred_at DESC);
CREATE INDEX activity_actor_ix   ON activity_event (workspace_id, actor_id, occurred_at DESC);

CREATE TABLE audit_event (
    id             uuid NOT NULL,
    workspace_id   uuid NOT NULL,
    event_type     text NOT NULL,
    actor_id       uuid,
    actor_type     text NOT NULL,   -- USER | SERVICE_ACCOUNT | PLUGIN | SYSTEM
    target_type    text,
    target_id      uuid,
    changes        jsonb NOT NULL DEFAULT '{}',
    request_id     uuid,
    -- The thread tying a user action to every effect it caused (docs/46).
    correlation_id uuid,
    ip_address     inet,            -- retained for incident investigation (ADR-025)
    user_agent     text,
    occurred_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (id, occurred_at)
) PARTITION BY RANGE (occurred_at);

CREATE INDEX audit_ws_ix   ON audit_event (workspace_id, occurred_at DESC);
CREATE INDEX audit_type_ix ON audit_event (workspace_id, event_type, occurred_at DESC);

-- Bootstrap partitions. The retention worker creates the next month ahead of
-- time and drops expired ones (docs/46 runbooks).
CREATE TABLE activity_event_default PARTITION OF activity_event DEFAULT;
CREATE TABLE audit_event_default    PARTITION OF audit_event    DEFAULT;
