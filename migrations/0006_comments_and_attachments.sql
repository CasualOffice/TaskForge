-- 0006 — Comments and attachments. See docs/28-ATTACHMENT-PIPELINE.md.

CREATE TABLE comment (
    id                uuid PRIMARY KEY,
    workspace_id      uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    task_id           uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    parent_comment_id uuid REFERENCES comment(id),   -- one level of threading
    author_id         uuid NOT NULL REFERENCES user_account(id),
    body              text NOT NULL CHECK (length(body) <= 65536),
    mentions          uuid[] NOT NULL DEFAULT '{}',  -- resolved at write time
    created_at        timestamptz NOT NULL DEFAULT now(),
    edited_at         timestamptz,
    deleted_at        timestamptz,
    version           bigint NOT NULL DEFAULT 1
);
CREATE INDEX comment_task_ix ON comment (task_id, created_at) WHERE deleted_at IS NULL;

CREATE TABLE attachment (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    task_id       uuid NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    object_key    text NOT NULL UNIQUE,           -- {workspace}/{task}/{attachment}
    filename      text NOT NULL,
    content_type  text NOT NULL,                  -- from magic bytes, not the client
    byte_size     bigint NOT NULL,
    checksum      text NOT NULL,                  -- sha256
    scan_status   text NOT NULL DEFAULT 'PENDING'
                  CHECK (scan_status IN ('PENDING','CLEAN','INFECTED','FAILED')),
    committed_at  timestamptz,                    -- NULL ⇒ invisible everywhere
    uploaded_by   uuid NOT NULL REFERENCES user_account(id),
    created_at    timestamptz NOT NULL DEFAULT now(),
    deleted_at    timestamptz
);
-- Partial on committed_at: uncommitted rows are not merely filtered at read
-- time, they are NOT IN THE INDEX READS USE. A forgotten WHERE clause cannot
-- leak an unscanned file (docs/28).
CREATE INDEX attachment_task_ix ON attachment (task_id)
    WHERE committed_at IS NOT NULL AND deleted_at IS NULL;
-- The orphan sweeper needs the complement.
CREATE INDEX attachment_pending_ix ON attachment (created_at) WHERE committed_at IS NULL;
