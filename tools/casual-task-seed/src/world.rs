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

fn build_workflows(
    sink: &mut Sink,
    det: &mut Det,
    plan: &Plan,
    workspace_id: Uuid,
) -> Vec<Workflow> {
    let mut out = Vec::with_capacity(plan.workflows);
    for (wi, (name, statuses)) in WORKFLOWS.iter().take(plan.workflows).enumerate() {
        let id = det.uuid_at(1_600_000_000_000 + wi as i64 * 1_000);
        sink.w(Table::Workflow)
            .row()
            .uuid(id)
            .uuid(workspace_id)
            .text(name)
            .bool(wi == 0)
            .int(1)
            .end();

        let mut built = Vec::with_capacity(statuses.len());
        let mut by_state: [Vec<usize>; 5] = std::array::from_fn(|_| Vec::new());
        for (si, (status_name, state)) in statuses.iter().enumerate() {
            let sid = det.uuid_at(1_600_000_000_000 + (wi * 100 + si) as i64 * 1_000);
            sink.w(Table::WorkflowStatus)
                .row()
                .uuid(sid)
                .uuid(id)
                .uuid(workspace_id)
                .text(status_name)
                .label(labels::state(*state))
                .int(si as i64 + 1)
                .bool(si == 0) // exactly one initial status per workflow
                .end();
            by_state[labels::state_index(*state)].push(si);
            built.push(Status {
                id: sid,
                name: status_name,
                state: *state,
            });
        }

        write_transitions(sink, det, (workspace_id, id), &built, wi == 0);
        out.push(Workflow {
            id,
            statuses: built,
            by_state,
        });
    }
    out
}

/// A forward and a backward edge between each consecutive pair, plus one
/// "from any status" edge into the terminal `CANCELED` status — the case the
/// nullable `from_status_id` exists for (migration 0004).
fn write_transitions(
    sink: &mut Sink,
    det: &mut Det,
    (workspace_id, workflow): (Uuid, Uuid),
    statuses: &[Status],
    is_default: bool,
) {
    let last = statuses.len() - 1; // the CANCELED status, always last
    let mut emit =
        |from: Option<Uuid>, to: Uuid, perm: &str, fields: &[&str], ignore_deps: bool| {
            sink.w(Table::WorkflowTransition)
                .row()
                .uuid(det.uuid_at(1_610_000_000_000))
                .uuid(workflow)
                .uuid(workspace_id)
                .opt_uuid(from)
                .uuid(to)
                .text(perm)
                .text_array(fields)
                .bool(ignore_deps)
                .end();
        };

    for i in 0..last.saturating_sub(1) {
        let (a, b) = (&statuses[i], &statuses[i + 1]);
        // Closing needs `task.close`; leaving a terminal status needs
        // `task.reopen` (docs/23 §The default workflow).
        let closing = b.state == TaskState::Completed;
        let required: &[&str] = if is_default && closing {
            &["resolution"]
        } else {
            &[]
        };
        emit(
            Some(a.id),
            b.id,
            if closing {
                "task.close"
            } else {
                "task.transition"
            },
            required,
            false,
        );
        emit(
            Some(b.id),
            a.id,
            if closing {
                "task.reopen"
            } else {
                "task.transition"
            },
            &[],
            false,
        );
    }

    // Cancelling ignores blockers: work nobody will finish should not be held
    // open by a dependency on other work nobody will finish.
    emit(None, statuses[last].id, "task.transition", &[], true);
}

fn build_projects(
    sink: &mut Sink,
    det: &mut Det,
    plan: &Plan,
    workflows: &[Workflow],
    (users, teams): (&[User], &[Uuid]),
    (workspace_id, now): (Uuid, i64),
) -> Vec<Project> {
    // Its own sub-stream, so the size distribution is a property of the seed
    // alone and can be asserted directly (see the reference-shape test below)
    // without generating two million rows to look at it.
    let sizes = project_sizes(&mut det.substream("project-sizes", 0), plan);
    let mut keys = HashSet::new();
    let mut projects = Vec::with_capacity(plan.projects);

    for (i, size) in sizes.iter().copied().enumerate() {
        let created = now - det.range(90, 1_000) * DAY_MS;
        let id = det.uuid_at(created);
        let base = vocab::PROJECT_KEYS[i % vocab::PROJECT_KEYS.len()];
        let key = unique_key(base, &mut keys);
        let name = format!("{} {}", title_case(base), det.pick(vocab::PROJECT_SUFFIXES));
        let workflow = if det.chance(650) {
            0
        } else {
            det.below(workflows.len() as u64) as usize
        };
        // Some projects belong to no team; `project.team_id` is nullable and a
        // corpus where it never is would leave that join path untested.
        let team = (!teams.is_empty() && !det.chance(150)).then(|| *det.pick(teams));
        let visibility = match det.weighted(&[10, 70, 20]) {
            0 => Visibility::Private,
            1 => Visibility::Team,
            _ => Visibility::Workspace,
        };
        let archived = det.chance(30).then(|| now - det.range(1, 200) * DAY_MS);
        let creator = det.below(users.len() as u64) as usize;
        let description = det.chance(700).then(|| {
            format!(
                "Tracks the {} owned by the {} team.",
                det.pick(vocab::COMPONENTS),
                det.pick(vocab::TEAM_NAMES)
            )
        });

        // Membership size tracks project size: a 100,000-task project with
        // three members would make every "who can see this" query
        // unrealistically cheap, and would starve the grant generator below of
        // distinct (user, role, project) combinations to write.
        let member_target = (size / 200).clamp(3, 60).min(users.len());
        let members = {
            let mut set = HashSet::new();
            set.insert(creator);
            while set.len() < member_target {
                set.insert(det.below(users.len() as u64) as usize);
            }
            let mut v: Vec<usize> = set.into_iter().collect();
            v.sort_unstable();
            v
        };
        let updater = users[*det.pick(&members)].id;

        sink.w(Table::Project)
            .row()
            .uuid(id)
            .uuid(workspace_id)
            .opt_uuid(team)
            .text(&key)
            .text(&name)
            .opt_text(description.as_deref())
            .label(labels::visibility(visibility))
            .uuid(workflows[workflow].id)
            .int(size as i64)
            .ts(created)
            .uuid(users[creator].id)
            .ts(now - det.range(0, 90) * DAY_MS)
            .opt_uuid(Some(updater))
            .int(1 + det.range(0, 40))
            .opt_ts(archived)
            .null()
            .end();

        for m in &members {
            sink.w(Table::ProjectMembership)
                .row()
                .uuid(id)
                .uuid(users[*m].id)
                .uuid(workspace_id)
                .ts(created + det.range(0, 60) * DAY_MS)
                .end();
        }

        let environments = build_environments(sink, det, (workspace_id, id));
        let milestones = build_milestones(sink, det, (workspace_id, id), (created, now));

        projects.push(Project {
            id,
            key,
            workflow,
            members,
            environments,
            milestones,
            created_at: created,
            task_count: size,
            tags: Vec::new(),
        });
    }
    projects
}

fn build_environments(
    sink: &mut Sink,
    det: &mut Det,
    (workspace_id, project): (Uuid, Uuid),
) -> Vec<Uuid> {
    let n = det.range(2, 5) as usize;
    let mut out = Vec::with_capacity(n);
    for (i, name) in vocab::ENVIRONMENTS.iter().take(n).enumerate() {
        let id = det.uuid_at(1_620_000_000_000);
        sink.w(Table::ProjectEnvironment)
            .row()
            .uuid(id)
            .uuid(project)
            .uuid(workspace_id)
            .text(name)
            .int(i as i64 + 1)
            .end();
        out.push(id);
    }
    out
}

fn build_milestones(
    sink: &mut Sink,
    det: &mut Det,
    (workspace_id, project): (Uuid, Uuid),
    (created, now): (i64, i64),
) -> Vec<Uuid> {
    let n = det.range(0, 9) as usize;
    let prefix = *det.pick(vocab::MILESTONE_PREFIXES);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let id = det.uuid_at(created);
        let due = created + (i as i64 + 1) * 45 * DAY_MS;
        let completed = (due < now && det.chance(800)).then(|| due + det.range(-5, 12) * DAY_MS);
        sink.w(Table::Milestone)
            .row()
            .uuid(id)
            .uuid(workspace_id)
            .uuid(project)
            .text(&format!("{prefix} {}", i + 1))
            .ts(due)
            .opt_ts(completed)
            .end();
        out.push(id);
    }
    out
}

/// Distribute `plan.tasks` across `plan.projects` by size class, then hand the
/// rounding remainder to the largest project so the total is exact — the corpus
/// is quoted as "2,000,000 tasks" and should contain exactly that.
fn project_sizes(det: &mut Det, plan: &Plan) -> Vec<usize> {
    if plan.projects == 0 {
        return Vec::new();
    }
    let shares: Vec<u32> = PROJECT_SIZE_CLASSES
        .iter()
        .map(|(share, _)| *share)
        .collect();
    let weights: Vec<u64> = (0..plan.projects)
        .map(|_| {
            let class = det.weighted(&shares);
            // ±20% jitter so two projects in the same class are not identical.
            u64::from(PROJECT_SIZE_CLASSES[class].1) * det.range(80, 121) as u64
        })
        .collect();
    let total: u64 = weights.iter().sum();
    let mut sizes: Vec<usize> = weights
        .iter()
        .map(|w| (u128::from(*w) * plan.tasks as u128 / u128::from(total)) as usize)
        .collect();
    let assigned: usize = sizes.iter().sum();
    let biggest = weights
        .iter()
        .enumerate()
        .max_by_key(|(_, w)| **w)
        .map(|(i, _)| i)
        .unwrap_or(0);
    sizes[biggest] += plan.tasks.saturating_sub(assigned);
    sizes
}

/// Project keys are immutable and appear in commit messages (ADR-007), so they
/// must satisfy `^[A-Z][A-Z0-9]{1,9}$` and be unique per workspace.
fn unique_key(base: &str, used: &mut HashSet<String>) -> String {
    let mut key = base.to_string();
    let mut n = 1;
    while used.contains(&key) {
        n += 1;
        let suffix = n.to_string();
        let keep = base.len().min(10 - suffix.len());
        key = format!("{}{suffix}", &base[..keep]);
    }
    used.insert(key.clone());
    key
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// Few tags, many tasks each — the shape a workspace converges on, and the one
/// that makes `task_tag_rev_ix` matter. Workspace tags carry `project_id NULL`,
/// which is exactly the case the `NULLS NOT DISTINCT` unique constraint in
/// migration 0005 exists to keep unique.
fn build_tags(
    sink: &mut Sink,
    det: &mut Det,
    plan: &Plan,
    workspace_id: Uuid,
    projects: &mut [Project],
) -> Vec<Uuid> {
    let mut workspace_tags = Vec::with_capacity(plan.workspace_tags);
    for name in vocab::TAG_NAMES.iter().take(plan.workspace_tags) {
        let id = det.uuid_at(1_630_000_000_000);
        let color = *det.pick(vocab::TAG_COLORS);
        sink.w(Table::Tag)
            .row()
            .uuid(id)
            .uuid(workspace_id)
            .null()
            .text(name)
            .opt_text(Some(color))
            .end();
        workspace_tags.push(id);
    }

    for p in projects.iter_mut() {
        let n = det.range(0, 5) as usize;
        let mut used = HashSet::new();
        for _ in 0..n {
            let name = *det.pick(vocab::TAG_NAMES);
            if !used.insert(name) {
                continue; // `tag` is unique per (workspace, project, name)
            }
            let id = det.uuid_at(1_640_000_000_000);
            let color = *det.pick(vocab::TAG_COLORS);
            sink.w(Table::Tag)
                .row()
                .uuid(id)
                .uuid(workspace_id)
                .uuid(p.id)
                .text(name)
                .opt_text(Some(color))
                .end();
            p.tags.push(id);
        }
    }
    workspace_tags
}

struct GrantCtx<'a> {
    workspace_id: Uuid,
    roles: &'a [Uuid],
    users: &'a [User],
    teams: &'a [Uuid],
    projects: &'a [Project],
    now: i64,
}

/// `role_assignment` is the only source of authority in the system (migration
/// 0003), so the corpus carries a realistic *shape* of grants and not only a
/// count: a few workspace-wide grants, one or two managers per project, and a
/// long tail of project-scoped members — including grants to teams, which is
/// what makes principal expansion cost anything at all.
/// Returns how many grants were written, which is **not** always
/// `plan.role_assignments`.
///
/// The tail draws distinct `(project, member)` pairs, and the number available
/// is `sum over projects of member_target` — a function of the project-size
/// distribution, not of the plan. At `--scale small` that space is 240 pairs
/// against a plan of 400, so the loop exhausts its attempt budget and returns
/// short. It used to return short *silently*, and `role_assignment` cardinality
/// is exactly what the `permission_resolution_cold` and `accessible_projects`
/// load-test cases measure — so a reader of that corpus would have been
/// measuring an authorization graph 20% thinner than the manifest implied.
fn build_role_assignments(
    sink: &mut Sink,
    det: &mut Det,
    plan: &Plan,
    ctx: &GrantCtx<'_>,
) -> usize {
    let mut seen: HashSet<(u8, Uuid, Uuid, u8, Uuid)> = HashSet::new();
    let mut written = 0usize;
    let granter = ctx.users[0].id;

    let mut emit = |det: &mut Det,
                    ptype: u8,
                    principal: Uuid,
                    role: usize,
                    scope: (u8, Uuid),
                    constraints: &str|
     -> bool {
        let role_id = ctx.roles[role];
        if !seen.insert((ptype, principal, role_id, scope.0, scope.1)) {
            return false; // the UNIQUE that makes granting idempotent
        }
        let granted = ctx.now - det.range(1, 900) * DAY_MS;
        sink.w(Table::RoleAssignment)
            .row()
            .uuid(det.uuid_at(granted))
            .uuid(ctx.workspace_id)
            .label(match ptype {
                0 => "USER",
                1 => "TEAM",
                _ => "SERVICE_ACCOUNT",
            })
            .uuid(principal)
            .uuid(role_id)
            .label(match scope.0 {
                0 => "WORKSPACE",
                1 => "TEAM",
                2 => "PROJECT",
                _ => "ENVIRONMENT",
            })
            .uuid(scope.1)
            .json(constraints)
            .uuid(granter)
            .ts(granted)
            .end();
        true
    };

    for u in ctx.users.iter().take(2) {
        written += usize::from(emit(det, 0, u.id, 0, (0, ctx.workspace_id), "{}"));
    }
    for u in ctx.users.iter().skip(2).take(4) {
        written += usize::from(emit(det, 0, u.id, 1, (0, ctx.workspace_id), "{}"));
    }

    for p in ctx.projects {
        for _ in 0..det.range(1, 3) {
            let u = *det.pick(&p.members);
            written += usize::from(emit(det, 0, ctx.users[u].id, 2, (2, p.id), "{}"));
        }
    }

    for t in ctx.teams {
        for _ in 0..det.range(1, 4) {
            if ctx.projects.is_empty() {
                break;
            }
            let p = det.pick(ctx.projects).id;
            written += usize::from(emit(det, 1, *t, 3, (2, p), "{}"));
        }
    }

    // The long tail: individual project members, plus guests carrying the
    // `not_external` constraint from docs/04. Bounded so an exhausted
    // combination space cannot spin.
    let mut attempts = 0;
    while written < plan.role_assignments && attempts < plan.role_assignments * 50 + 1_000 {
        attempts += 1;
        if ctx.projects.is_empty() {
            break;
        }
        let p = det.pick(ctx.projects);
        let u = *det.pick(&p.members);
        let (role, constraints) = if ctx.users[u].guest {
            (4, r#"{"not_external":true}"#)
        } else {
            (3, "{}")
        };
        let (pid, uid) = (p.id, ctx.users[u].id);
        written += usize::from(emit(det, 0, uid, role, (2, pid), constraints));
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale::Scale;

    /// The size distribution the reference corpus will have, asserted without
    /// generating it: `project_sizes` draws from its own sub-stream, so these
    /// are the numbers a real `--scale reference` run produces.
    ///
    /// Checked against `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md`
    /// §Reference capacity: 2,000,000 tasks, 200 projects, p95 around 20,000
    /// per project, and nothing near the 200,000 design ceiling.
    #[test]
    fn reference_project_sizes_match_the_capacity_table() {
        let plan = Plan::for_scale(Scale::Reference);
        let mut det = Det::stream(20_260_101, "world").substream("project-sizes", 0);
        let mut sizes = project_sizes(&mut det, &plan);

        assert_eq!(
            sizes.iter().sum::<usize>(),
            2_000_000,
            "the corpus is quoted as two million tasks and must contain exactly that"
        );
        assert_eq!(sizes.len(), 200);

        sizes.sort_unstable();
        let p95 = sizes[sizes.len() * 95 / 100];
        let max = *sizes.last().expect("200 projects");
        assert!(
            (12_000..32_000).contains(&p95),
            "p95 tasks per project is {p95}; docs/30 says 20,000"
        );
        assert!(max < 200_000, "largest project {max} exceeds the ceiling");
        assert!(
            max > 8 * sizes[sizes.len() / 2],
            "distribution is not skewed enough: max {max}, median {}",
            sizes[sizes.len() / 2]
        );
        assert!(sizes[0] > 0, "every project has tasks");
    }

    /// Keys are immutable and appear in commit messages (ADR-007), so a
    /// collision is not a cosmetic problem.
    #[test]
    fn project_keys_stay_unique_and_legal() {
        let mut used = HashSet::new();
        for i in 0..500 {
            let key = unique_key(
                vocab::PROJECT_KEYS[i % vocab::PROJECT_KEYS.len()],
                &mut used,
            );
            assert!((2..=10).contains(&key.len()), "{key} violates the CHECK");
            let mut chars = key.chars();
            assert!(chars.next().is_some_and(|c| c.is_ascii_uppercase()));
            assert!(chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        }
        assert_eq!(used.len(), 500, "every key must be distinct");
    }

    #[test]
    fn every_workflow_has_one_initial_status_and_all_five_states() {
        for (_, statuses) in WORKFLOWS {
            let mut seen = [false; 5];
            for (_, state) in *statuses {
                seen[labels::state_index(*state)] = true;
            }
            assert!(
                seen.iter().all(|s| *s),
                "a workflow must be able to express every state, or the task \
                 generator cannot give a task the status its state requires"
            );
            assert_eq!(
                statuses.last().map(|(_, s)| *s),
                Some(TaskState::Canceled),
                "the transition builder assumes CANCELED is last"
            );
        }
    }

    /// Owner is a superset of every other template, and Guest grants nothing
    /// that writes (docs/04 §Built-in role templates).
    #[test]
    fn role_templates_are_ordered_by_power() {
        for p in permission::ALL {
            let key = p.as_str();
            assert!(role_has(0, key), "Owner must hold {key}");
            for role in 1..5 {
                if role_has(role, key) {
                    assert!(
                        role_has(role - 1, key),
                        "role {role} exceeds role {}",
                        role - 1
                    );
                }
            }
        }
        assert!(!role_has(4, "task.update"), "Guest must not write");
        assert!(role_has(4, "task.read"));
    }
}
