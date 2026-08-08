-- 0001 — Extensions and closed enum types.
-- See docs/22-DATABASE-SCHEMA.md §Types.
--
-- Enums are Postgres enum types where the set is closed forever, and
-- text + CHECK where it may grow. Adding an enum value is cheap; removing one
-- is not — that asymmetry decides which is used.

CREATE EXTENSION IF NOT EXISTS pg_trgm;      -- fuzzy / substring search (docs/26)
CREATE EXTENSION IF NOT EXISTS btree_gin;    -- composite GIN over scalar + tsvector
CREATE EXTENSION IF NOT EXISTS citext;       -- case-insensitive email and tag names

-- The permanent semantic contract (docs/23). Adding a sixth value is a breaking
-- API change requiring a major version and a superseding ADR.
CREATE TYPE task_state      AS ENUM ('BACKLOG','PLANNED','ACTIVE','COMPLETED','CANCELED');

CREATE TYPE task_type       AS ENUM ('TASK','BUG','FEATURE','INCIDENT','REQUEST');

-- Ordered so `ORDER BY priority DESC` and `priority >= 'HIGH'` are semantic and
-- index-served, rather than a CASE expression (docs/27).
CREATE TYPE task_priority   AS ENUM ('NONE','LOW','MEDIUM','HIGH','URGENT');

CREATE TYPE visibility      AS ENUM ('PRIVATE','TEAM','WORKSPACE');
CREATE TYPE principal_type  AS ENUM ('USER','TEAM','SERVICE_ACCOUNT');

-- TASK scope is deliberately absent (ADR-005): per-task grants make the grant
-- table scale with task count and break the one-resolution-per-list property.
CREATE TYPE scope_type      AS ENUM ('WORKSPACE','TEAM','PROJECT','ENVIRONMENT');

-- One dependency kind in v1 (ADR-019). Relates/duplicates are presentational
-- links, not dependencies, and are deferred.
CREATE TYPE dependency_kind AS ENUM ('BLOCKS');
