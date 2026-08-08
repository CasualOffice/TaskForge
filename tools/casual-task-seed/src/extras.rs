//! The operational tables around the work items: saved views, notifications,
//! automation rules, service accounts, tokens, plugin installations, the
//! outbox, and the audit stream.
//!
//! These are small relative to `task` and `activity_event`, but leaving them
//! empty would be a quiet lie: the unread-badge count, the outbox poll, and the
//! audit page are all gated operations in
//! `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md`, and each is served by a
//! partial index that only means something once its table has rows on both
//! sides of the predicate.

use uuid::Uuid;

use crate::copy::{Sink, Table};
use crate::det::{DAY_MS, Det};
use crate::scale::Plan;
use crate::tasks::json_string;
use crate::vocab;
use crate::world::World;

/// Notification reasons, ranked, from `docs/29-NOTIFICATIONS-AND-DELIVERY.md`
/// §Reasons, not events. Weighted by how often each is the *highest* applicable
/// reason, since only one notification per user per event is written.
const REASONS: [(&str, u32); 6] = [
    ("MENTIONED", 120),
    ("ASSIGNED", 300),
    ("REPORTED", 220),
    ("SUBSCRIBED", 90),
    ("PARTICIPATED", 220),
    ("TEAM", 50),
];

const NOTIFICATION_EVENTS: [&str; 5] = [
    "task.assigned",
    "task.status.changed",
    "comment.created",
    "task.closed",
    "task.updated",
];

pub fn generate(sink: &mut Sink, world: &World, plan: &Plan, seed: u64, tasks: &[Uuid]) {
    let mut det = Det::stream(seed, "extras");
    let accounts = service_accounts(sink, &mut det, world, plan);
    api_tokens(sink, &mut det, world, &accounts);
    plugins(sink, &mut det, world, plan);
    saved_views(sink, &mut det, world, plan);
    automation_rules(sink, &mut det, world);
    notifications(sink, &mut det, world, plan, tasks);
    outbox(sink, &mut det, world, plan, tasks);
    audit(sink, &mut det, world, plan, &accounts);
}

fn service_accounts(sink: &mut Sink, det: &mut Det, world: &World, plan: &Plan) -> Vec<Uuid> {
    let mut ids = Vec::with_capacity(plan.service_accounts);
    for i in 0..plan.service_accounts {
        let created = world.now - det.range(30, 800) * DAY_MS;
        let id = det.uuid_at(created);
        let disabled = det
            .chance(150)
            .then(|| world.now - det.range(1, 60) * DAY_MS);
        sink.w(Table::ServiceAccount)
            .row()
            .uuid(id)
            .uuid(world.workspace_id)
            .text(&format!(
                "ci-{}",
                vocab::TEAM_NAMES[i % vocab::TEAM_NAMES.len()].to_lowercase()
            ))
            .uuid(world.users[0].id)
            .opt_ts(disabled)
            .end();
        ids.push(id);
    }
    ids
}

/// Tokens are stored as argon2id hashes: the plaintext is shown once and is
/// unrecoverable, so a database dump is not a credential dump (docs/40). The
/// corpus stores hash-shaped strings that are not hashes of anything, which is
/// the only safe thing a seed generator can do.
fn api_tokens(sink: &mut Sink, det: &mut Det, world: &World, accounts: &[Uuid]) {
    let mut emit = |det: &mut Det, principal_type: &str, principal: Uuid, i: usize| {
        let created = world.now - det.range(10, 700) * DAY_MS;
        let hash = format!(
            "$argon2id$v=19$m=19456,t=2,p=1${}${}",
            det.hex(16),
            det.hex(32)
        );
        let expires = det.chance(600).then(|| created + 365 * DAY_MS);
        let revoked = det
            .chance(120)
            .then(|| created + det.range(1, 300) * DAY_MS);
        let used = det
            .chance(800)
            .then(|| world.now - det.range(0, 40) * DAY_MS);
        sink.w(Table::ApiToken)
            .row()
            .uuid(det.uuid_at(created))
            .uuid(world.workspace_id)
            .label(principal_type)
            .uuid(principal)
            .text(&hash)
            .text(&format!("token-{i}"))
            .opt_ts(used)
            .opt_ts(expires)
            .opt_ts(revoked)
            .end();
    };

    for (i, a) in accounts.iter().enumerate() {
        emit(det, "SERVICE_ACCOUNT", *a, i);
    }
    for i in 0..world.users.len() / 10 {
        let u = world.users[det.below(world.users.len() as u64) as usize].id;
        emit(det, "USER", u, 1_000 + i);
    }
}

fn plugins(sink: &mut Sink, det: &mut Det, world: &World, plan: &Plan) {
    // `min` silently truncates; PLUGIN_IDS holds exactly what the reference
    // plan asks for, so any increase would shrink the corpus without a word.
    assert!(
        plan.plugins <= vocab::PLUGIN_IDS.len(),
        "scale {} asks for {} plugin installations and vocab::PLUGIN_IDS holds {}",
        plan.scale.as_str(),
        plan.plugins,
        vocab::PLUGIN_IDS.len()
    );
    for i in 0..plan.plugins.min(vocab::PLUGIN_IDS.len()) {
        let installed = world.now - det.range(20, 600) * DAY_MS;
        let scopes: Vec<&str> = [
            "task:read",
            "task:write",
            "comment:write",
            "webhook:receive",
        ][..det.range(1, 5) as usize]
            .to_vec();
        let uninstalled = det
            .chance(150)
            .then(|| world.now - det.range(1, 20) * DAY_MS);
        sink.w(Table::PluginInstallation)
            .row()
            .uuid(det.uuid_at(installed))
            .uuid(world.workspace_id)
            .text(vocab::PLUGIN_IDS[i])
            .text(&format!("{}.{}.0", det.range(1, 4), det.range(0, 12)))
            .text(&det.hex(32))
            .text_array(&scopes)
            .json(r#"{"notify":true}"#)
            .text(&format!("vault://taskforge/plugin/{i}"))
            .uuid(world.users[0].id)
            .ts(installed)
            .bool(uninstalled.is_none())
            .opt_ts(uninstalled)
            .end();
    }
}

/// Saved views use the AST from `docs/27-FILTER-AND-SAVED-VIEW-DSL.md` §The AST,
/// including its symbolic values — `@me` is what makes one stored view correct
/// for every user who opens it.
fn saved_views(sink: &mut Sink, det: &mut Det, world: &World, plan: &Plan) {
    const FILTERS: [(&str, &str, &str); 4] = [
        (
            "My overdue work",
            r#"{"op":"and","clauses":[{"field":"assignee","op":"eq","value":"@me"},{"field":"due_at","op":"before","value":"@today"},{"field":"state","op":"in","value":["ACTIVE","PLANNED"]}]}"#,
            "LIST",
        ),
        (
            "Urgent bugs",
            r#"{"op":"and","clauses":[{"field":"type","op":"eq","value":"BUG"},{"field":"priority","op":"gte","value":"HIGH"}]}"#,
            "BOARD",
        ),
        (
            "Unassigned backlog",
            r#"{"op":"and","clauses":[{"field":"assignee","op":"is_empty"},{"field":"state","op":"eq","value":"BACKLOG"}]}"#,
            "TABLE",
        ),
        (
            "Security follow-up",
            r#"{"op":"and","clauses":[{"field":"tag","op":"in","value":["security"]},{"field":"archived","op":"eq","value":false}]}"#,
            "LIST",
        ),
    ];
    const SORTS: [&str; 3] = [
        r#"[{"field":"due_at","dir":"asc"}]"#,
        r#"[{"field":"priority","dir":"desc"},{"field":"updated_at","dir":"desc"}]"#,
        r#"[{"field":"position","dir":"asc"}]"#,
    ];

    for i in 0..plan.saved_views() {
        let (name, filter, layout) = FILTERS[i % FILTERS.len()];
        let owner = &world.users[det.below(world.users.len() as u64) as usize];
        let created = world.now - det.range(1, 800) * DAY_MS;
        // Workspace-wide when `project_id` is NULL (docs/27 §Saved views).
        let project = (!world.projects.is_empty() && det.chance(600))
            .then(|| world.projects[det.below(world.projects.len() as u64) as usize].id);
        sink.w(Table::SavedView)
            .row()
            .uuid(det.uuid_at(created))
            .uuid(world.workspace_id)
            .opt_uuid(project)
            .uuid(owner.id)
            .text(name)
            .json(filter)
            .json(SORTS[det.below(3) as usize])
            .label(layout)
            .bool(det.chance(300))
            .ts(created)
            .int(1)
            .end();
    }
}

/// One rule per project or so. Every rule executes as a **named** principal
/// (`run_as`), never as the triggering user — rule authoring is otherwise a
/// privilege-escalation primitive (docs/36).
fn automation_rules(sink: &mut Sink, det: &mut Det, world: &World) {
    for p in &world.projects {
        if !det.chance(700) {
            continue;
        }
        let run_as = world.users[*det.pick(&p.members)].id;
        sink.w(Table::AutomationRule)
            .row()
            .uuid(det.uuid_at(p.created_at + DAY_MS))
            .uuid(world.workspace_id)
            .opt_uuid(Some(p.id))
            .text("Auto-assign urgent bugs to the on-call")
            .json(r#"{"event":"task.created"}"#)
            .json(
                r#"{"op":"and","clauses":[{"field":"type","op":"eq","value":"BUG"},{"field":"priority","op":"gte","value":"HIGH"}]}"#,
            )
            .json(
                r#"[{"type":"assign","to":"@oncall"},{"type":"add_tag","tag":"triage"}]"#,
            )
            .bool(det.chance(850))
            .uuid(run_as)
            .int(1)
            .end();
    }
}

/// Notifications, with a genuine unread minority: `notification_unread_ix` is a
/// partial index and its whole point is that the badge count is an index-only
/// scan over a small fraction of the table (docs/29).
fn notifications(sink: &mut Sink, det: &mut Det, world: &World, plan: &Plan, tasks: &[Uuid]) {
    let reason_weights: Vec<u32> = REASONS.iter().map(|(_, w)| *w).collect();
    for _ in 0..plan.notifications() {
        let at = world.now - det.range(0, 400) * DAY_MS;
        let user = &world.users[det.below(world.users.len() as u64) as usize];
        let actor = &world.users[det.below(world.users.len() as u64) as usize];
        let aggregate = (!tasks.is_empty()).then(|| tasks[det.below(tasks.len() as u64) as usize]);
        let event = NOTIFICATION_EVENTS[det.below(NOTIFICATION_EVENTS.len() as u64) as usize];
        let reason = REASONS[det.weighted(&reason_weights)].0;
        let read = det.chance(720).then(|| at + det.range(1, 72) * 3_600_000);
        sink.w(Table::Notification)
            .row()
            .uuid(det.uuid_at(at))
            .uuid(world.workspace_id)
            .uuid(user.id)
            .text(event)
            .text(reason)
            .opt_uuid(aggregate)
            // The payload carries the actor's display name rather than only an
            // id: a notification is rendered long after the fact, and resolving
            // names at read time is the N+1 the feed cannot afford (docs/29).
            .json(&format!(
                r#"{{"aggregate_type":"task","event":{},"reason":{},"actor":{{"display_name":{}}}}}"#,
                json_string(event),
                json_string(reason),
                json_string(&actor.display_name)
            ))
            .ts(at)
            .opt_ts(read)
            .end();
    }
}

/// The outbox is a queue, not a log. Dispatched rows are pruned, so the table
/// stays small however large the corpus is; what has to be present is the
/// *shape* — a pending head, a dispatched tail, and a couple of rows that have
/// failed enough times to be in the dead-letter index (docs/25 §Dispatch).
fn outbox(sink: &mut Sink, det: &mut Det, world: &World, plan: &Plan, tasks: &[Uuid]) {
    for i in 0..plan.outbox_events() {
        let at = world.now - det.range(0, 3) * DAY_MS - det.range(0, 86_400) * 1_000;
        let aggregate = if tasks.is_empty() {
            world.workspace_id
        } else {
            tasks[det.below(tasks.len() as u64) as usize]
        };
        let event = NOTIFICATION_EVENTS[det.below(NOTIFICATION_EVENTS.len() as u64) as usize];
        // A tenth are still pending, and a handful have exhausted their retries.
        let pending = det.chance(100);
        let dead = !pending && det.chance(20);
        let attempts = if dead {
            det.range(6, 12)
        } else if pending {
            det.range(0, 2)
        } else {
            1
        };
        sink.w(Table::OutboxEvent)
            .row()
            .uuid(det.uuid_at(at))
            .uuid(world.workspace_id)
            .text(event)
            .text("task")
            .uuid(aggregate)
            .json(&format!(
                r#"{{"schema_version":1,"aggregate_type":"task","event":{},"seq":{i}}}"#,
                json_string(event)
            ))
            .int(1)
            .ts(at)
            .opt_ts((!pending).then_some(at + 400))
            .int(attempts)
            .opt_text(dead.then_some("connection refused by subscriber endpoint"))
            .end();
    }
}

/// Audit is not a copy of activity. It records authorization, identity, and
/// configuration events — including denials, because a burst of them is the
/// clearest available signal of a compromised account and is invisible if only
/// successes are recorded (docs/25 §Audit specifics).
fn audit(sink: &mut Sink, det: &mut Det, world: &World, plan: &Plan, accounts: &[Uuid]) {
    const EVENTS: [(&str, &str, u32); 8] = [
        ("auth.login", "USER", 380),
        ("auth.login.failed", "USER", 90),
        ("permission.denied", "USER", 120),
        ("role.assigned", "USER", 70),
        ("role.revoked", "USER", 30),
        ("token.created", "SERVICE_ACCOUNT", 40),
        ("project.updated", "USER", 210),
        ("plugin.installed", "USER", 10),
    ];
    let weights: Vec<u32> = EVENTS.iter().map(|(_, _, w)| *w).collect();
    let count = (plan.tasks / 20).max(world.users.len() * 20);

    for _ in 0..count {
        let at = world.now - det.range(0, 730) * DAY_MS - det.range(0, 86_400) * 1_000;
        let (event, actor_type, _) = EVENTS[det.weighted(&weights)];
        let system = det.chance(30);
        let actor = if actor_type == "SERVICE_ACCOUNT" && !accounts.is_empty() {
            Some(accounts[det.below(accounts.len() as u64) as usize])
        } else if system {
            None
        } else {
            Some(world.users[det.below(world.users.len() as u64) as usize].id)
        };
        let (target_type, target) = match event {
            "role.assigned" | "role.revoked" => (
                "team",
                (!world.teams.is_empty())
                    .then(|| world.teams[det.below(world.teams.len() as u64) as usize]),
            ),
            "project.updated" => (
                "project",
                (!world.projects.is_empty())
                    .then(|| world.projects[det.below(world.projects.len() as u64) as usize].id),
            ),
            _ => (
                "user",
                Some(world.users[det.below(world.users.len() as u64) as usize].id),
            ),
        };
        let changes = match event {
            "permission.denied" => r#"{"permission":"task.delete","decision":"DENY"}"#,
            "project.updated" => r#"{"visibility":{"from":"TEAM","to":"WORKSPACE"}}"#,
            _ => "{}",
        };
        // TEST-NET-3 (RFC 5737). Retained for incident investigation
        // (ADR-025), and never a real address.
        let ip = format!("203.0.113.{}", det.below(254) + 1);

        sink.w(Table::AuditEvent)
            .row()
            .uuid(det.uuid_at(at))
            .uuid(world.workspace_id)
            .text(event)
            .opt_uuid(actor)
            .text(if system { "SYSTEM" } else { actor_type })
            .opt_text(Some(target_type))
            .opt_uuid(target)
            .json(changes)
            .opt_uuid(Some(det.uuid_at(at)))
            .opt_uuid(Some(det.uuid_at(at)))
            .opt_text(Some(&ip))
            .opt_text(Some(
                vocab::USER_AGENTS[det.below(vocab::USER_AGENTS.len() as u64) as usize],
            ))
            .ts(at)
            .end();
    }
}
