-- 0002 — Tenancy and identity. See docs/22 §Tenancy & identity, docs/32.

CREATE TABLE workspace (
    id            uuid PRIMARY KEY,
    name          text NOT NULL,
    slug          text NOT NULL UNIQUE,
    -- Bumped in the same transaction as any grant/membership change; part of
    -- every permission cache key, so a stale entry simply misses (ADR-012).
    authz_epoch   bigint NOT NULL DEFAULT 1,
    settings      jsonb NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now(),
    deleted_at    timestamptz
);

-- The only table without workspace_id: a person legitimately exists across
-- workspaces. Every read path reaches it through a membership (docs/32).
CREATE TABLE user_account (
    id            uuid PRIMARY KEY,
    email         citext UNIQUE,               -- NULL once anonymized (ADR-026)
    display_name  text NOT NULL,
    avatar_url    text,
    is_tombstone  boolean NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE workspace_membership (
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES user_account(id),
    member_type   text NOT NULL CHECK (member_type IN ('MEMBER','GUEST')),
    joined_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX workspace_membership_user_ix ON workspace_membership (user_id);

CREATE TABLE team (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    name          text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    deleted_at    timestamptz,
    UNIQUE (workspace_id, name)
);

CREATE TABLE team_membership (
    team_id       uuid NOT NULL REFERENCES team(id) ON DELETE CASCADE,
    user_id       uuid NOT NULL REFERENCES user_account(id),
    PRIMARY KEY (team_id, user_id)
);
-- Both directions: principal expansion needs teams-of-user (docs/26).
CREATE INDEX team_membership_user_ix ON team_membership (user_id);
