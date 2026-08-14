//! The structural half of the corpus: one workspace, its people, its
//! authorization graph, its workflows, and its projects.
//!
//! Everything here is small enough to hold in memory (500 users, 200 projects
//! at reference scale) and all of it is needed by the task generator, which
//! streams and therefore cannot look anything up after the fact.

use std::collections::HashSet;

use casual_task_model::{TaskState, Visibility, permission};
use uuid::Uuid;

use crate::copy::{Sink, Table};
use crate::det::{DAY_MS, Det};
use crate::labels;
use crate::scale::Plan;
use crate::vocab;

#[derive(Debug)]
pub struct User {
    pub id: Uuid,
    pub display_name: String,
    pub guest: bool,
}

#[derive(Debug)]
pub struct Status {
    pub id: Uuid,
    pub name: &'static str,
    pub state: TaskState,
}

#[derive(Debug)]
pub struct Workflow {
    pub id: Uuid,
    pub statuses: Vec<Status>,
    /// Status indices grouped by state, so the task generator can pick a status
    /// that *is* the state it decided on. `task.state` is derived from the
    /// status and written in the same statement precisely so the two cannot
    /// drift (migration 0005); a corpus that violated that would make every
    /// state-filtered measurement meaningless.
    pub by_state: [Vec<usize>; 5],
}

#[derive(Debug)]
pub struct Project {
    pub id: Uuid,
    pub key: String,
    pub workflow: usize,
    pub members: Vec<usize>,
    pub environments: Vec<Uuid>,
    pub milestones: Vec<Uuid>,
    pub tags: Vec<Uuid>,
    pub created_at: i64,
    pub task_count: usize,
}

#[derive(Debug)]
pub struct World {
    pub workspace_id: Uuid,
    pub now: i64,
    pub users: Vec<User>,
    pub teams: Vec<Uuid>,
    pub workflows: Vec<Workflow>,
    pub projects: Vec<Project>,
    /// Workspace-scoped tags, most popular first.
    pub workspace_tags: Vec<Uuid>,
    /// Ways this corpus fell short of its plan. Empty is the normal case;
    /// anything here is printed by `main` rather than left for a reader to
    /// discover by comparing counts.
    pub notes: Vec<String>,
}

/// Status sets for the five shipped workflows. The first is the default from
/// `docs/23-WORKFLOW-AND-STATE-MACHINE.md` §The default workflow; the rest are
/// shapes real teams configure. The terminal `CANCELED` status is always last,
/// which the transition builder relies on.
const WORKFLOWS: &[(&str, &[(&str, TaskState)])] = &[
    (
        "Default",
        &[
            ("Backlog", TaskState::Backlog),
            ("Todo", TaskState::Planned),
            ("In Progress", TaskState::Active),
            ("Code Review", TaskState::Active),
            ("Blocked", TaskState::Active),
            ("Ready for QA", TaskState::Active),
            ("Done", TaskState::Completed),
            ("Canceled", TaskState::Canceled),
        ],
    ),
    (
        "Support",
        &[
            ("Triage", TaskState::Backlog),
            ("Scheduled", TaskState::Planned),
            ("Investigating", TaskState::Active),
            ("Waiting on Customer", TaskState::Active),
            ("Resolved", TaskState::Completed),
            ("Won't Do", TaskState::Canceled),
        ],
    ),
    (
        "Delivery",
        &[
            ("Icebox", TaskState::Backlog),
            ("Ready", TaskState::Planned),
            ("Building", TaskState::Active),
            ("Verifying", TaskState::Active),
            ("Shipped", TaskState::Completed),
            ("Dropped", TaskState::Canceled),
        ],
    ),
    (
        "Incident",
        &[
            ("Reported", TaskState::Backlog),
            ("Acknowledged", TaskState::Planned),
            ("Mitigating", TaskState::Active),
            ("Monitoring", TaskState::Active),
            ("Closed", TaskState::Completed),
            ("False Alarm", TaskState::Canceled),
        ],
    ),
    (
        "Research",
        &[
            ("Proposed", TaskState::Backlog),
            ("Accepted", TaskState::Planned),
            ("In Study", TaskState::Active),
            ("Documented", TaskState::Completed),
            ("Abandoned", TaskState::Canceled),
        ],
    ),
];

const ROLES: [&str; 5] = [
    "Owner",
    "Administrator",
    "Project Manager",
    "Member",
    "Guest",
];

/// Project size classes: `(share per mille, relative weight)`.
///
/// Uniform project sizes are the most misleading thing a synthetic corpus can
/// do — every index looks selective when every project holds the same 10,000
/// rows, and the plan that appears at 100,000 never shows up. This table gives
/// a handful of very large projects, a long tail of small ones, and a p95 in
/// the region of the 20,000 tasks-per-project figure in
/// `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md` §Reference capacity.
const PROJECT_SIZE_CLASSES: [(u32, u32); 4] = [(20, 90), (80, 18), (250, 8), (650, 3)];

pub fn build(sink: &mut Sink, plan: &Plan, seed: u64, now: i64) -> World {
    let mut det = Det::stream(seed, "world");
    let workspace_created = now - 1_095 * DAY_MS;
    let workspace_id = det.uuid_at(workspace_created);

    write_workspace(sink, workspace_id, workspace_created, plan);
    let users = build_users(sink, &mut det, plan, workspace_id, now);
    let teams = build_teams(
        sink,
        &mut det,
        plan,
        workspace_id,
        &users,
        workspace_created,
    );
    let roles = build_roles(sink, &mut det, workspace_id, workspace_created);
    let workflows = build_workflows(sink, &mut det, plan, workspace_id);
    let mut projects = build_projects(
        sink,
        &mut det,
        plan,
        &workflows,
        (&users, &teams),
        (workspace_id, now),
    );
    let workspace_tags = build_tags(sink, &mut det, plan, workspace_id, &mut projects);

    let role_assignments = build_role_assignments(
        sink,
        &mut det,
        plan,
        &GrantCtx {
            workspace_id,
            roles: &roles,
            users: &users,
            teams: &teams,
            projects: &projects,
            now,
        },
    );
    let mut notes = Vec::new();
    if role_assignments < plan.role_assignments {
        notes.push(format!(
            "role_assignment: {role_assignments} written against a plan of {}. The \
             distinct (project, member) pairs this scale produces are fewer than the \
             plan asks for; the corpus is valid but its authorization graph is thinner \
             than the plan implies, which is what permission_resolution_cold and \
             accessible_projects measure.",
            plan.role_assignments
        ));
    }

    World {
        workspace_id,
        now,
        users,
        teams,
        workflows,
        projects,
        workspace_tags,
        notes,
    }
}

fn write_workspace(sink: &mut Sink, id: Uuid, created: i64, plan: &Plan) {
    sink.w(Table::Workspace)
        .row()
        .uuid(id)
        .text("TaskForge Reference Workspace")
        .text(&format!("reference-{}", plan.scale.as_str()))
        .int(1)
        .json(r#"{"timezone":"UTC","week_start":"MON"}"#)
        .ts(created)
        .null()
        .end();
}

fn build_users(
    sink: &mut Sink,
    det: &mut Det,
    plan: &Plan,
    workspace_id: Uuid,
    now: i64,
) -> Vec<User> {
    let mut users = Vec::with_capacity(plan.users);
    for i in 0..plan.users {
        let created = now - det.range(30, 1_090) * DAY_MS;
        let id = det.uuid_at(created);
        let first = vocab::FIRST_NAMES[i % vocab::FIRST_NAMES.len()];
        let last =
            vocab::LAST_NAMES[(i / vocab::FIRST_NAMES.len() + i * 7) % vocab::LAST_NAMES.len()];
        let display_name = format!("{first} {last}");
        // ~1% are anonymized tombstones: email NULL, which citext UNIQUE
        // permits repeatedly, and which is the ADR-026 end state. Keeping a few
        // means every join to `user_account` is exercised against a row whose
        // email is absent.
        //
        // The last user is always one. At 1% of 25 users the tiny corpus would
        // otherwise contain none, and a CI corpus that omits a shape is a CI
        // corpus that cannot catch a regression in handling it.
        let tombstone = det.chance(10) || i + 1 == plan.users;
        let email = format!(
            "{}.{}{i}@example.test",
            first.to_lowercase(),
            last.to_lowercase()
        );
        let avatar = det
            .chance(600)
            .then(|| format!("https://avatars.example.test/{i}.png"));
        let guest = det.chance(80);
        let updated = created + det.range(0, 400) * DAY_MS;

        sink.w(Table::UserAccount)
            .row()
            .uuid(id)
            .opt_text(if tombstone { None } else { Some(&email) })
            .text(if tombstone {
                "Deactivated user"
            } else {
                &display_name
            })
            .opt_text(avatar.as_deref())
            .bool(tombstone)
            .ts(created)
            .ts(updated)
            .end();

        sink.w(Table::WorkspaceMembership)
            .row()
            .uuid(workspace_id)
            .uuid(id)
            .label(if guest { "GUEST" } else { "MEMBER" })
            .ts(created)
            .end();

        users.push(User {
            id,
            display_name,
            guest,
        });
    }
    users
}

fn build_teams(
    sink: &mut Sink,
    det: &mut Det,
    plan: &Plan,
    workspace_id: Uuid,
    users: &[User],
    created: i64,
) -> Vec<Uuid> {
    let mut teams = Vec::with_capacity(plan.teams);
    for i in 0..plan.teams {
        let at = created + i as i64 * DAY_MS;
        let id = det.uuid_at(at);
        sink.w(Table::Team)
            .row()
            .uuid(id)
            .uuid(workspace_id)
            .text(vocab::TEAM_NAMES[i % vocab::TEAM_NAMES.len()])
            .ts(at)
            .null()
            .end();

        // Team size is skewed too: principal expansion during permission
        // resolution costs one row per team the actor belongs to
        // (`docs/04-RBAC-AND-AUTHORIZATION.md` §Caching).
        let target = (users.len() / plan.teams).max(2) + det.range(0, 6) as usize;
        let mut members = HashSet::new();
        for _ in 0..target {
            members.insert(det.below(users.len() as u64) as usize);
        }
        let mut members: Vec<usize> = members.into_iter().collect();
        members.sort_unstable();
        for m in members {
            sink.w(Table::TeamMembership)
                .row()
                .uuid(id)
                .uuid(users[m].id)
                .end();
        }
        teams.push(id);
    }
    teams
}

/// The five built-in role templates from `docs/04-RBAC-AND-AUTHORIZATION.md`
/// §Built-in role templates.
///
/// The permission set comes from `casual-task-model::permission::ALL`, which the
/// parity test already holds equal to migration 0011's `permission` table. A
/// hand-written list here would be a third copy, and the one that drifts.
fn build_roles(sink: &mut Sink, det: &mut Det, workspace_id: Uuid, created: i64) -> Vec<Uuid> {
    let mut ids = Vec::with_capacity(ROLES.len());
    for (i, name) in ROLES.iter().enumerate() {
        let id = det.uuid_at(created + i as i64 * 1_000);
        sink.w(Table::Role)
            .row()
            .uuid(id)
            .uuid(workspace_id)
            .text(name)
            .bool(true)
            .ts(created)
            .ts(created)
            .int(1)
            .end();

        for p in permission::ALL {
            if role_has(i, p.as_str()) {
                sink.w(Table::RolePermission)
                    .row()
                    .uuid(id)
                    .text(p.as_str())
                    .end();
            }
        }
        ids.push(id);
    }
    ids
}

fn role_has(role: usize, key: &str) -> bool {
    match role {
        0 => true, // Owner — everything, including workspace.delete.
        1 => !matches!(key, "workspace.delete" | "workspace.owner"),
        2 => {
            // Project Manager — full control inside scoped projects, but no
            // workspace configuration and no project creation or deletion.
            (key.starts_with("project.") || key.starts_with("task.") || key == "tag.manage")
                && !matches!(key, "project.create" | "project.delete")
        }
        3 => matches!(
            key,
            "task.read"
                | "task.create"
                | "task.update"
                | "task.assign"
                | "task.transition"
                | "task.close"
                | "task.comment"
                | "task.history.read"
                | "task.attachment.create"
                | "task.attachment.read"
        ),
        _ => matches!(
            key,
            "task.read" | "task.comment" | "task.history.read" | "task.attachment.read"
        ),
    }
}

include!("world_projects.rs");
#[cfg(test)]
#[path = "world_tests.rs"]
mod tests;
