# TaskForge — 02 Domain RBAC Workflow

## 4. Domain Model
- Workspace is the tenant boundary. A user joins through a workspace membership and may belong to multiple workspaces.
- Project is the primary collaboration boundary. Projects can be private, team-visible or workspace-visible.
- Task is the universal work item. Important fields include key, title, description, type, priority, status, state, reporter, assignees, project, environment, milestone, parent, dates, tags, version, archive and deletion timestamps.
- Environment is an optional project context such as Development, QA, Staging, Production, Customer UAT or Region EU.
- Tag is a reusable workspace- or project-scoped label. Tags are many-to-many with tasks.
- History is an append-only stream of user-readable activity. Audit is a more security-focused stream with additional request and authentication metadata.

## 5. State, Status and Workflow
- State is a stable semantic category used by APIs, analytics and plugins: BACKLOG, PLANNED, ACTIVE, COMPLETED and CANCELED.
- Status is configurable and maps to one state. Examples: Ready for Development maps to PLANNED; In Progress and Code Review map to ACTIVE; Done maps to COMPLETED.
- A workflow contains statuses and allowed transitions. Every transition can require a permission, validation rules, mandatory fields, dependency checks or automation hooks.
- Default workflow: Backlog → Todo → In Progress → Done, with Blocked as an ACTIVE status and Canceled as a terminal CANCELED status.
- Closing a task requires both task.close permission and a valid transition to a COMPLETED status. Reopening requires task.reopen and a permitted transition out of a terminal status.

## 6. Roles and Permission Architecture
- Roles are administrator-defined collections of stable permissions. Built-in templates are Owner, Administrator, Project Manager, Member and Guest. Administrators can clone templates and create custom roles such as QA Reviewer or Release Manager.
- A role assignment targets a principal—user, team, group or service account—and a scope: WORKSPACE, TEAM, PROJECT, ENVIRONMENT, with TASK reserved for exceptional sharing.
- A person can be Project Manager in one project and Guest in another. Project-level role assignment is therefore fundamental.
- Permission examples: project.create, project.update, project.member.manage, project.role.assign, project.workflow.manage, task.create, task.update, task.assign, task.move, task.close, task.reopen, task.delete, task.comment, task.history.read, tag.manage, plugin.install and automation.manage.
- Authorization evaluates actor, permission, resource, workspace and contextual constraints. Initial constraint support should be deliberately small: assignee-only, reporter-only, project member, environment, ownership and external-user restrictions.
- Privilege escalation protections: users cannot delegate permissions they lack authority to grant; project managers cannot grant workspace roles; the last owner cannot be removed; plugin permissions require explicit consent; role changes are audited.

## 7. Permission Examples
- Create task: actor must have task.create in the target project.
- Assign task: actor must have task.assign, and the assignee must be an eligible project member unless an administrator overrides policy.
- Close task: actor must have task.close and workflow validation must allow the transition.
- Reopen task: actor must have task.reopen and a valid reverse transition.
- Update own assigned tasks only: assign a role with task.update plus an assignee-is-actor constraint.
- Manage project roles: require project.role.assign and block assignment of permissions beyond the delegator’s grant ceiling.
