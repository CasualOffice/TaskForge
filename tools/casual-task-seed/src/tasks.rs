//! Tasks and everything hanging off them: assignees, tags, dependencies,
//! comments, attachments, the search projection, and the activity stream.
//!
//! This is the only part of the corpus that streams. At reference scale it
//! writes 2,000,000 tasks and roughly 20,000,000 activity events, so nothing
//! larger than one project is ever held in memory — a project's task list is
//! kept only because `parent_id` and `task_dependency` must point at rows that
//! have already been written, which is what makes every foreign key satisfiable
//! in a single forward pass of `COPY`.

use casual_task_model::{TaskState, TaskType};
use uuid::Uuid;

use crate::copy::{Sink, Table};
use crate::det::{DAY_MS, Det, base36};
use crate::labels;
use crate::vocab;
use crate::world::{Project, Workflow, World};

/// Tuned so the reference corpus lands near the 20,000,000 activity events in
/// `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md` §Reference capacity — about
/// ten events per task. Every other contributor to that total (assignments,
/// comments, transitions) is fixed by its own distribution, so this is the one
/// free parameter and therefore the one that carries the adjustment.
const ACTIVITY_EDIT_P: u64 = 420;

/// Percent of a project's tasks treated as recent. Everything older draws from
/// the settled distribution, which is overwhelmingly terminal: a mature
/// workspace is mostly finished work, and a corpus that is half open makes
/// every `state`-filtered index look far more selective than it is.
const RECENT_SHARE: usize = 15;

/// `(BACKLOG, PLANNED, ACTIVE, COMPLETED, CANCELED)` weights. Together with
/// `RECENT_SHARE` these put roughly 70% of tasks in a terminal state.
const STATE_WEIGHTS_SETTLED: [u32; 5] = [40, 70, 90, 680, 120];
const STATE_WEIGHTS_RECENT: [u32; 5] = [210, 250, 350, 150, 40];

const TYPE_WEIGHTS: [u32; 5] = [550, 250, 120, 30, 50];
const PRIORITY_WEIGHTS: [u32; 5] = [300, 200, 300, 150, 50];
const PRIORITY_WEIGHTS_URGENT: [u32; 5] = [20, 60, 200, 420, 300];

/// Assignees per task: mostly one, sometimes none, occasionally a pair — the
/// distribution the partial unique index on the primary flag exists for
/// (ADR-010).
const ASSIGNEE_WEIGHTS: [u32; 4] = [120, 680, 150, 50];
const ASSIGNEE_WEIGHTS_BACKLOG: [u32; 4] = [700, 250, 40, 10];

const TAG_COUNT_WEIGHTS: [u32; 5] = [250, 350, 250, 110, 40];

/// One already-written task, remembered only until its project is finished.
#[derive(Debug, Clone, Copy)]
struct Row {
    id: Uuid,
    created_at: i64,
    state: TaskState,
    has_parent: bool,
}

/// What the task pass produced. `ids` is a thin, evenly spaced sample — the
/// operational tables (notifications, outbox) reference real tasks, and keeping
/// two million identifiers alive to achieve that would defeat the point of
/// streaming.
#[derive(Debug, Default)]
pub struct Generated {
    pub tasks: u64,
    pub ids: Vec<Uuid>,
}

pub fn generate(sink: &mut Sink, world: &World, seed: u64) -> Generated {
    let root = Det::stream(seed, "tasks");
    // Zipf-shaped tag popularity, computed once: the first few tags carry most
    // of the corpus, which is what gives `task_tag_rev_ix` posting lists a
    // realistic length spread instead of a uniform one.
    let tag_weights: Vec<u32> = (0..world.workspace_tags.len())
        .map(|i| (4_000 / (i as u32 + 1)).max(1))
        .collect();

    let mut out = Generated::default();
    for (pi, project) in world.projects.iter().enumerate() {
        let mut det = root.substream("project", pi as u64);
        out.tasks += generate_project(sink, world, project, &tag_weights, &mut det, &mut out.ids);
    }
    out
}

fn generate_project(
    sink: &mut Sink,
    world: &World,
    project: &Project,
    tag_weights: &[u32],
    det: &mut Det,
    sample: &mut Vec<Uuid>,
) -> u64 {
    let workflow = &world.workflows[project.workflow];
    let n = project.task_count;
    if n == 0 {
        return 0;
    }
    let span = (world.now - project.created_at).max(DAY_MS);
    let step = (span / n as i64).max(1);
    let recent_from = n - (n * RECENT_SHARE / 100).clamp(1, n);
    let sample_stride = (n / 100).max(1);

    let mut rows: Vec<Row> = Vec::with_capacity(n);
    for i in 0..n {
        // Creation time advances monotonically with the task number: a task key
        // is read as a chronology, and a corpus where number and time disagree
        // would hand `task_list_ix` a correlation it does not have in practice.
        // The arithmetic goes through i128 because `span * i` overflows i64 at
        // reference scale.
        let offset = (i128::from(span) * i as i128 / n as i128) as i64;
        let created = (project.created_at + offset + det.below(step as u64) as i64).min(world.now);
        let id = det.uuid_at(created);

        let weights = if i >= recent_from {
            &STATE_WEIGHTS_RECENT
        } else {
            &STATE_WEIGHTS_SETTLED
        };
        let state = TaskState::ALL[det.weighted(weights)];
        let status = pick_status(det, workflow, state);

        let age = (world.now - created).max(1);
        let updated = if state.is_terminal() {
            created + det.below(age as u64) as i64
        } else {
            created + age * det.range(50, 101) / 100
        };

        let task_type = labels::TASK_TYPES[det.weighted(&TYPE_WEIGHTS)];
        let urgent_kind = matches!(task_type, TaskType::Incident | TaskType::Bug);
        let priority = labels::PRIORITIES[det.weighted(if urgent_kind {
            &PRIORITY_WEIGHTS_URGENT
        } else {
            &PRIORITY_WEIGHTS
        })];

        let reporter = world.users[*det.pick(&project.members)].id;
        let environment = (!project.environments.is_empty()
            && det.chance(if urgent_kind { 600 } else { 200 }))
        .then(|| *det.pick(&project.environments));
        let milestone = (!project.milestones.is_empty() && det.chance(350))
            .then(|| *det.pick(&project.milestones));

        // One level of nesting only. Deeper trees exist, but the parent must
        // already be in the file for the self-referencing foreign key to hold
        // inside a single COPY stream, and a corpus full of deep trees would
        // exercise a shape the product does not encourage.
        let parent = (i > 0 && det.chance(120))
            .then(|| {
                let candidate = det.below(i as u64) as usize;
                (!rows[candidate].has_parent).then_some(rows[candidate].id)
            })
            .flatten();

        let start_at = (state != TaskState::Backlog && det.chance(400))
            .then(|| created + det.range(0, 10) * DAY_MS);
        let due_at = due_date(det, state, updated, world.now);
        let title = title(det);
        let description = det.chance(600).then(|| description(det));
        let deleted = det.chance(5).then(|| updated + DAY_MS);
        let archived =
            (state == TaskState::Completed && updated < world.now - 90 * DAY_MS && det.chance(60))
                .then(|| updated + 60 * DAY_MS);
        let edits = det.geometric(ACTIVITY_EDIT_P, 12);
        let position = rank(det, i);
        let updated_by = world.users[*det.pick(&project.members)].id;

        sink.w(Table::Task)
            .row()
            .uuid(id)
            .uuid(world.workspace_id)
            .uuid(project.id)
            .int(i as i64 + 1)
            .text(&title)
            .opt_text(description.as_deref())
            .label(labels::task_type(task_type))
            .label(labels::priority(priority))
            .uuid(workflow.statuses[status].id)
            .label(labels::state(state))
            .uuid(reporter)
            .opt_uuid(environment)
            .opt_uuid(milestone)
            .opt_uuid(parent)
            .opt_ts(start_at)
            .opt_ts(due_at)
            .text(&position)
            .ts(created)
            .uuid(reporter)
            .ts(updated)
            .opt_uuid(Some(updated_by))
            .int(i64::from(edits) + 1)
            .opt_ts(archived)
            .opt_ts(deleted)
            .end();

        let task = TaskCtx {
            id,
            project,
            state,
            status,
            created,
            updated,
            title: &title,
            task_type,
        };
        let assignees = write_assignees(sink, det, world, &task);
        write_tags(sink, det, world, &task, tag_weights);
        if i > 0 && det.chance(80) {
            let blocker = rows[det.below(i as u64) as usize];
            write_dependency(sink, det, world, &blocker, id);
        }
        let comments = write_comments(sink, det, world, &task);
        write_attachments(sink, det, world, &task);
        if deleted.is_none() {
            write_search(sink, world, &task);
        }
        write_activity(
            sink,
            det,
            world,
            &task,
            ActivityShape {
                assignees,
                comments,
                edits,
            },
        );

        if i % sample_stride == 0 && deleted.is_none() {
            sample.push(id);
        }
        rows.push(Row {
            id,
            created_at: created,
            state,
            has_parent: parent.is_some(),
        });
    }
    rows.len() as u64
}

fn pick_status(det: &mut Det, workflow: &Workflow, state: TaskState) -> usize {
    let candidates = &workflow.by_state[labels::state_index(state)];
    if candidates.is_empty() {
        return 0;
    }
    *det.pick(candidates)
}

/// Due dates cluster around the corpus clock, with a real overdue tail: "My
/// Work" and every default board filter sort on `due_at`, and dates spread
/// uniformly over three years would make `task_due_ix` look far more selective
/// than it is.
fn due_date(det: &mut Det, state: TaskState, updated: i64, now: i64) -> Option<i64> {
    if !det.chance(550) {
        return None;
    }
    if state.is_terminal() {
        // Finished work was due around the time it finished.
        return Some(updated + det.range(-20, 10) * DAY_MS);
    }
    // Open work: about a third of it is already overdue.
    let width = match det.weighted(&[600, 250, 150]) {
        0 => 30,
        1 => 180,
        _ => 540,
    };
    Some(now + det.range(-width / 3, width) * DAY_MS)
}

/// Lexicographic board rank (ADR-013). Zero-padded base-36 sorts the same way
/// lexicographically as it does numerically, and the occasionally appended
/// character is what a rank looks like after a card has been dragged between
/// two others — still under the 32-character compaction threshold in docs/30.
fn rank(det: &mut Det, i: usize) -> String {
    let mut r = base36(1_000_000 + i as u64 * 64, 6);
    if det.chance(100) {
        r.push_str(&base36(det.below(36), 1));
    }
    r
}

struct TaskCtx<'a> {
    id: Uuid,
    project: &'a Project,
    state: TaskState,
    status: usize,
    created: i64,
    updated: i64,
    title: &'a str,
    task_type: TaskType,
}

struct ActivityShape {
    assignees: u32,
    comments: u32,
    edits: u32,
}

fn write_assignees(sink: &mut Sink, det: &mut Det, world: &World, task: &TaskCtx<'_>) -> u32 {
    let weights = if task.state == TaskState::Backlog {
        &ASSIGNEE_WEIGHTS_BACKLOG
    } else {
        &ASSIGNEE_WEIGHTS
    };
    let n = det.weighted(weights);
    let mut chosen: Vec<usize> = Vec::with_capacity(n);
    for _ in 0..n {
        let u = *det.pick(&task.project.members);
        if !chosen.contains(&u) {
            chosen.push(u); // the (task_id, user_id) primary key
        }
    }
    for (k, u) in chosen.iter().enumerate() {
        sink.w(Table::TaskAssignee)
            .row()
            .uuid(task.id)
            .uuid(world.users[*u].id)
            .uuid(world.workspace_id)
            // At most one primary, enforced by a partial unique index
            // (ADR-010). The corpus has to satisfy it, not merely avoid it.
            .bool(k == 0)
            .ts(task.created + det.range(0, 3) * DAY_MS)
            .end();
    }
    chosen.len() as u32
}

fn write_tags(sink: &mut Sink, det: &mut Det, world: &World, task: &TaskCtx<'_>, weights: &[u32]) {
    let n = det.weighted(&TAG_COUNT_WEIGHTS);
    if n == 0 || world.workspace_tags.is_empty() {
        return;
    }
    let mut chosen: Vec<Uuid> = Vec::with_capacity(n);
    for _ in 0..n {
        let tag = if !task.project.tags.is_empty() && det.chance(250) {
            *det.pick(&task.project.tags)
        } else {
            world.workspace_tags[det.weighted(weights)]
        };
        if !chosen.contains(&tag) {
            chosen.push(tag); // the (task_id, tag_id) primary key
        }
    }
    for tag in chosen {
        sink.w(Table::TaskTag)
            .row()
            .uuid(task.id)
            .uuid(tag)
            .uuid(world.workspace_id)
            .end();
    }
}

/// `BLOCKS` always runs from the older task to the newer one. That is not
/// cosmetic: it makes the dependency graph acyclic by construction, so the
/// corpus cannot contain the cycle the transition path rejects under an
/// advisory lock at runtime (`docs/24-CONCURRENCY-AND-INTEGRITY.md`).
fn write_dependency(sink: &mut Sink, det: &mut Det, world: &World, blocker: &Row, task: Uuid) {
    // Abandoned work does not block anything; the transition path would treat
    // such an edge as permanently unresolved.
    if blocker.id == task || blocker.state == TaskState::Canceled {
        return;
    }
    sink.w(Table::TaskDependency)
        .row()
        .uuid(blocker.id)
        .uuid(task)
        .uuid(world.workspace_id)
        .label("BLOCKS")
        .ts(blocker.created_at + det.range(0, 5) * DAY_MS)
        .end();
}

fn write_comments(sink: &mut Sink, det: &mut Det, world: &World, task: &TaskCtx<'_>) -> u32 {
    // Half of all tasks are never discussed; the rest have a long tail, which
    // puts the p95 near the 20 comments-per-task figure in docs/30.
    if !det.chance(500) {
        return 0;
    }
    let n = det.geometric(900, 60);
    if n == 0 {
        return 0;
    }
    // Only *roots* are eligible parents. Collecting every prior comment here
    // instead would thread replies onto replies: `parent_comment_id` has no
    // depth constraint in migration 0006, so the corpus would load happily and
    // the product would be measured against a shape it does not allow. The task
    // generator makes the same distinction (see `has_parent` above).
    let mut roots: Vec<Uuid> = Vec::with_capacity(n as usize);
    let window = (task.updated - task.created).max(1);
    for k in 0..n {
        let at = task.created + window * i64::from(k + 1) / i64::from(n + 1);
        let id = det.uuid_at(at);
        let author = world.users[*det.pick(&task.project.members)].id;
        // One level of threading (migration 0006), and the parent must already
        // be in the file.
        let parent = (!roots.is_empty() && det.chance(200)).then(|| *det.pick(&roots));
        let mentions: Vec<Uuid> = if det.chance(150) {
            vec![world.users[*det.pick(&task.project.members)].id]
        } else {
            Vec::new()
        };
        let body = format!(
            "{} {}",
            det.pick(vocab::COMMENT_OPENERS),
            det.pick(vocab::COMMENT_BODIES)
        );
        let edited = det.chance(80).then(|| at + det.range(1, 48) * 3_600_000);
        let deleted = det.chance(20).then(|| at + 2 * DAY_MS);

        sink.w(Table::Comment)
            .row()
            .uuid(id)
            .uuid(world.workspace_id)
            .uuid(task.id)
            .opt_uuid(parent)
            .uuid(author)
            .text(&body)
            .uuid_array(&mentions)
            .ts(at)
            .opt_ts(edited)
            .opt_ts(deleted)
            .int(if edited.is_some() { 2 } else { 1 })
            .end();
        if parent.is_none() {
            roots.push(id);
        }
    }
    n
}

fn write_attachments(sink: &mut Sink, det: &mut Det, world: &World, task: &TaskCtx<'_>) {
    if !det.chance(80) {
        return;
    }
    for _ in 0..det.range(1, 4) {
        let at = task.created + det.range(0, 10) * DAY_MS;
        let id = det.uuid_at(at);
        let which = det.below(vocab::ATTACHMENT_NAMES.len() as u64) as usize;
        // ~5% are uploaded but never committed. Those rows are invisible to
        // every read path because `attachment_task_ix` excludes them (docs/28);
        // without them the orphan sweeper's index has nothing to find.
        let committed = det.chance(950).then_some(at + 30_000);
        let scan = match det.weighted(&[20, 950, 5, 25]) {
            0 => "PENDING",
            1 => "CLEAN",
            2 => "INFECTED",
            _ => "FAILED",
        };
        let object_key = format!("{}/{}/{}", world.workspace_id, task.id, id);
        let checksum = det.hex(32);
        let size = det.range(1_024, 8_388_608);
        let uploader = world.users[*det.pick(&task.project.members)].id;

        sink.w(Table::Attachment)
            .row()
            .uuid(id)
            .uuid(world.workspace_id)
            .uuid(task.id)
            .text(&object_key)
            .text(vocab::ATTACHMENT_NAMES[which])
            .text(vocab::CONTENT_TYPES[which])
            .int(size)
            .text(&checksum)
            .label(scan)
            .opt_ts(committed)
            .uuid(uploader)
            .ts(at)
            .null()
            .end();
    }
}

include!("task_events.rs");
