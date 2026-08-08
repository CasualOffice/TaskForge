-- 0015 — `role.assign`, distinct from `role.manage` (D-049, docs/04).
--
-- Assigning a role and authoring one are different authorities.
--
-- The closed set had `project.role.assign` for assigning inside a project and
-- only `role.manage` above it — which is also the AUTHORING permission. So a
-- workspace-level assigner had to hold the right to author roles as well, and
-- anyone who could assign could therefore mint a role with more power than
-- their own and grant it to themselves.
--
-- The escalation ceiling in `casual-task-authz` already forbids granting a
-- permission the actor does not hold, so the hole was narrow. It sat, however,
-- precisely where the most privileged actors are, and closing it by splitting
-- the permission is cheaper now than after roles carry it in customer data.
--
-- A permission is added to a CLOSED set, so this is a design change and not a
-- detail: docs/04 records it and casual-task-model's registry is the other half
-- (a permission in one and not the other fails the schema gate).

INSERT INTO permission (key, description, added_in) VALUES
  ('role.assign', 'Assign existing roles (workspace scope)', 'v1')
ON CONFLICT (key) DO NOTHING;
