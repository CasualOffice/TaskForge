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
    // Exactly at the limit today: range(2, 5) tops out at 4 and ENVIRONMENTS
    // holds 4. Widening the range without adding names would silently cap.
    assert!(
        n <= vocab::ENVIRONMENTS.len(),
        "asked for {n} environments and vocab::ENVIRONMENTS holds {}",
        vocab::ENVIRONMENTS.len()
    );
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
    // `take` would silently hand back fewer tags than the plan asked for, and
    // `tasks::generate` derives its Zipf tag-weight table from the length of
    // what comes back — so a shortfall would propagate into task_tag
    // cardinality with nothing to notice it. TAG_NAMES has exactly as many
    // entries as the reference plan wants, i.e. no headroom at all.
    assert!(
        plan.workspace_tags <= vocab::TAG_NAMES.len(),
        "scale {} asks for {} workspace tags and vocab::TAG_NAMES holds {}. Add \
         names, or the corpus is quietly smaller than its plan.",
        plan.scale.as_str(),
        plan.workspace_tags,
        vocab::TAG_NAMES.len()
    );
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

include!("world_grants.rs");
