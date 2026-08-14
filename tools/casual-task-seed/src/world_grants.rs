/// Populate the corpus authority graph and return the number of grants.
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
                    principal_type: u8,
                    principal: Uuid,
                    role: usize,
                    scope: (u8, Uuid),
                    constraints: &str|
     -> bool {
        let role_id = ctx.roles[role];
        if !seen.insert((principal_type, principal, role_id, scope.0, scope.1)) {
            return false;
        }
        let granted = ctx.now - det.range(1, 900) * DAY_MS;
        sink.w(Table::RoleAssignment)
            .row()
            .uuid(det.uuid_at(granted))
            .uuid(ctx.workspace_id)
            .label(match principal_type {
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

    for user in ctx.users.iter().take(2) {
        written += usize::from(emit(det, 0, user.id, 0, (0, ctx.workspace_id), "{}"));
    }
    for user in ctx.users.iter().skip(2).take(4) {
        written += usize::from(emit(det, 0, user.id, 1, (0, ctx.workspace_id), "{}"));
    }
    for project in ctx.projects {
        for _ in 0..det.range(1, 3) {
            let user = *det.pick(&project.members);
            written += usize::from(emit(det, 0, ctx.users[user].id, 2, (2, project.id), "{}"));
        }
    }
    for team in ctx.teams {
        for _ in 0..det.range(1, 4) {
            if ctx.projects.is_empty() {
                break;
            }
            let project = det.pick(ctx.projects).id;
            written += usize::from(emit(det, 1, *team, 3, (2, project), "{}"));
        }
    }

    let mut attempts = 0;
    while written < plan.role_assignments && attempts < plan.role_assignments * 50 + 1_000 {
        attempts += 1;
        if ctx.projects.is_empty() {
            break;
        }
        let project = det.pick(ctx.projects);
        let user = *det.pick(&project.members);
        let (role, constraints) = if ctx.users[user].guest {
            (4, r#"{"not_external":true}"#)
        } else {
            (3, "{}")
        };
        written += usize::from(emit(
            det,
            0,
            ctx.users[user].id,
            role,
            (2, project.id),
            constraints,
        ));
    }
    written
}
