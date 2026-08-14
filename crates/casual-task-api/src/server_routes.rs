/// Build the router.
///
/// **Every route is registered before the layers, and must stay that way.** In
/// axum a route added *after* `.layer()` is not wrapped by it — so a route
/// appended to the returned `Router` silently escapes both the CSRF guard and
/// the request id. `docs/05` says "every unsafe method without a valid token is
/// rejected", and that holds only while this ordering does.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/metrics", get(metrics))
        .route(
            "/api/v1/auth/login",
            axum::routing::post(crate::auth::login),
        )
        .route(
            "/api/v1/auth/logout",
            axum::routing::post(crate::auth::logout),
        )
        .route("/api/v1/auth/session", get(crate::middleware::whoami))
        // MFA (C-001, docs/40 §MFA). All under /auth because they are about
        // the credential rather than about a workspace — the one exception is
        // the per-workspace requirement toggle, which lives with the workspace
        // it configures.
        .route(
            "/api/v1/auth/mfa",
            get(crate::mfa::status).delete(crate::mfa::disable),
        )
        .route(
            "/api/v1/auth/mfa/enrolment",
            axum::routing::post(crate::mfa::begin),
        )
        .route(
            "/api/v1/auth/mfa/enrolment/confirm",
            axum::routing::post(crate::mfa::confirm),
        )
        .route(
            "/api/v1/auth/mfa/step-up",
            axum::routing::post(crate::mfa::step_up),
        )
        .route(
            "/api/v1/auth/mfa/recovery",
            axum::routing::post(crate::mfa::verify_recovery_code),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/mfa-requirement",
            axum::routing::put(crate::mfa::set_requirement),
        )
        // Both reset routes are registered HERE, above the layers, like every
        // other route: one registered below them escapes the CSRF guard and the
        // request id. They pass the CSRF guard because they carry no session
        // cookie — there is nothing to forge with, which is the same reason
        // login does.
        .route(
            "/api/v1/auth/password-reset",
            axum::routing::post(crate::password_reset::request),
        )
        .route(
            "/api/v1/auth/password-reset/confirm",
            axum::routing::post(crate::password_reset::confirm),
        )
        // C-006 / C-008. Every one of these takes `WorkspaceMember`, which is
        // the only thing that mints an `AuthContext` — so none of them can
        // reach a tenant row without a validated membership (`docs/32`).
        .route(
            "/api/v1/projects",
            get(crate::projects::list).post(crate::projects::create),
        )
        .route(
            "/api/v1/projects/{id}",
            get(crate::projects::read).patch(crate::projects::update),
        )
        .route(
            "/api/v1/projects/{id}/tasks",
            axum::routing::post(crate::tasks::create),
        )
        .route(
            "/api/v1/tasks/{id}/attachments",
            get(crate::attachments::list).post(crate::attachments::presign),
        )
        .route(
            "/api/v1/attachments/{id}/commit",
            axum::routing::post(crate::attachments::commit),
        )
        .route(
            "/api/v1/attachments/{id}/download",
            get(crate::attachments::download),
        )
        .route("/api/v1/tasks", get(crate::tasks::list))
        // Static, so it is matched before `/tasks/{id}` — `bulk` is not a task
        // id and never reaches the read handler.
        .route(
            "/api/v1/tasks/bulk",
            axum::routing::post(crate::tasks::bulk),
        )
        // C-016. Both take `WorkspaceMember`, and both scope every statement to
        // the caller's own user id — a notification is the one tenant row whose
        // owner is not implied by the workspace.
        .route("/api/v1/workflows/{id}", get(crate::workflows::read))
        // Workflow authoring (`docs/23` §Editing a workflow). A status delete
        // carries `?migrate_to=` because a status holding tasks cannot simply
        // vanish — every task on it moves in the same transaction, attributed
        // to the admin who asked.
        .route(
            "/api/v1/workflows/{id}/statuses",
            get(crate::workflows::list_statuses).post(crate::workflows::create_status),
        )
        .route(
            "/api/v1/workflows/{id}/statuses/order",
            axum::routing::post(crate::workflows::reorder_statuses),
        )
        .route(
            "/api/v1/workflows/{id}/statuses/{sid}",
            axum::routing::patch(crate::workflows::update_status)
                .delete(crate::workflows::delete_status),
        )
        .route(
            "/api/v1/workflows/{id}/transitions",
            axum::routing::post(crate::workflows::create_transition),
        )
        .route(
            "/api/v1/workflows/{id}/transitions/{tid}",
            axum::routing::patch(crate::workflows::update_transition)
                .delete(crate::workflows::delete_transition),
        )
        // Environments. Also an authorization scope (`Scope::Environment`), so
        // these are part of the permission model and not merely a task field.
        // A project involves many teams (`docs/03`). Authority is
        // `project.member.manage`, evaluated against the project's EXISTING
        // teams — evaluating against the incoming one would let anyone holding
        // a grant on a team add that team to any project they can see.
        .route(
            "/api/v1/projects/{id}/teams",
            get(crate::project_teams::list).post(crate::project_teams::add),
        )
        .route(
            "/api/v1/projects/{id}/teams/{team_id}",
            axum::routing::delete(crate::project_teams::remove),
        )
        .route(
            "/api/v1/projects/{id}/environments",
            get(crate::environments::list).post(crate::environments::create),
        )
        .route(
            "/api/v1/projects/{id}/environments/order",
            axum::routing::put(crate::environments::reorder),
        )
        .route(
            "/api/v1/environments/{id}",
            axum::routing::patch(crate::environments::rename).delete(crate::environments::delete),
        )
        .route(
            "/api/v1/tasks/{id}/environment",
            axum::routing::put(crate::environments::set_on_task),
        )
        // Releases (`docs/45`). A release is what went out together, which is
        // the one fact neither the status board nor the environment view can
        // hold. Cutting one takes `task.update` — the same authority as the
        // promotions it batches, and deliberately not a key of its own.
        .route(
            "/api/v1/projects/{id}/releases",
            get(crate::releases::list).post(crate::releases::cut),
        )
        .route("/api/v1/releases/{id}", get(crate::releases::read))
        // Reports (ADR-027). A filter plus an aggregation, over the same closed
        // field set as every list — which is what keeps the index contract true
        // when reporting arrives instead of making it the exception.
        .route(
            "/api/v1/reports/run",
            axum::routing::post(crate::reports::run),
        )
        // Roles (`docs/04` §API). Authoring is workspace-scoped and is a
        // different permission from assigning (D-049) — anyone who could do
        // both could mint a role carrying more than they hold and grant it to
        // themselves.
        // A person's own account. Outside the tenant boundary by construction —
        // a person belongs to many workspaces — and every handler answers only
        // about the caller, which is what makes that safe.
        .route("/api/v1/me", get(crate::me::read).patch(crate::me::update))
        // Whose turn is it (`docs/45`). The home screen's whole answer.
        .route("/api/v1/me/queue", get(crate::custody::queue))
        // Which teams the caller is in. The sidebar's list, and the reason a
        // team is a place you can stand rather than a filter you must know the
        // id of (`docs/45`).
        .route("/api/v1/me/teams", get(crate::workspaces::teams::my_teams))
        .route(
            "/api/v1/me/password",
            axum::routing::post(crate::me::change_password),
        )
        .route(
            "/api/v1/me/sessions",
            get(crate::me::sessions).delete(crate::me::revoke_other_sessions),
        )
        .route(
            "/api/v1/me/sessions/{id}",
            axum::routing::delete(crate::me::revoke_session),
        )
        .route(
            "/api/v1/roles",
            get(crate::roles::list).post(crate::roles::create),
        )
        .route(
            "/api/v1/roles/{id}",
            axum::routing::patch(crate::roles::update),
        )
        .route(
            "/api/v1/role-assignments",
            get(crate::roles::list_assignments).post(crate::roles::assign),
        )
        .route(
            "/api/v1/role-assignments/{id}",
            axum::routing::delete(crate::roles::revoke),
        )
        .route(
            "/api/v1/permissions/effective",
            get(crate::permissions::effective),
        )
        .route(
            "/api/v1/permissions/explain",
            axum::routing::post(crate::permissions::explain),
        )
        .route("/api/v1/notifications", get(crate::notifications::list))
        .route(
            "/api/v1/notifications/read",
            axum::routing::post(crate::notifications::mark_read),
        )
        .route(
            "/api/v1/tasks/{id}",
            get(crate::tasks::read)
                .patch(crate::tasks::update)
                .delete(crate::tasks::delete),
        )
        // docs/23: the ONLY door to a status change. A `PATCH` naming
        // `status_id` is refused with TF-WFL-0001 and pointed here.
        .route(
            "/api/v1/tasks/{id}/transitions",
            axum::routing::post(crate::tasks::transition),
        )
        .route(
            "/api/v1/tasks/{id}/assignees",
            get(crate::tasks::assignees).post(crate::tasks::assign),
        )
        .route(
            "/api/v1/tasks/{id}/assignees/{user_id}",
            axum::routing::delete(crate::tasks::unassign),
        )
        // C-009 — comments. Visibility is decided by the task, never by the
        // comment: a comment carries no permission of its own.
        // C-011 — the History tab. Every change has written an activity record
        // in the same transaction as the change since C-011; this is the first
        // thing that reads them.
        .route("/api/v1/tasks/{id}/activity", get(crate::activity::stream))
        // The chain of custody (`docs/45`). Three commands and the one read that
        // renders them, because they are one panel and always read together.
        .route("/api/v1/tasks/{id}/custody", get(crate::custody::read))
        .route(
            "/api/v1/tasks/{id}/team",
            axum::routing::put(crate::custody::transfer),
        )
        .route(
            "/api/v1/tasks/{id}/promotions",
            axum::routing::post(crate::custody::promote),
        )
        .route(
            "/api/v1/tasks/{id}/verifications",
            axum::routing::post(crate::custody::verify),
        )
        // C-008 — the Relations panel. The write is docs/05's; the read shape
        // is chosen (see the module docs) because docs/05 specifies none.
        .route(
            "/api/v1/tasks/{id}/dependencies",
            get(crate::dependencies::read).post(crate::dependencies::add),
        )
        .route(
            "/api/v1/tasks/{id}/dependencies/{other_id}",
            axum::routing::delete(crate::dependencies::remove),
        )
        .route(
            "/api/v1/tasks/{id}/comments",
            get(crate::comments::thread).post(crate::comments::create),
        )
        .route(
            "/api/v1/comments/{id}",
            axum::routing::patch(crate::comments::edit),
        )
        .route(
            "/api/v1/tasks/{id}/tags",
            get(crate::tasks::tags_of).post(crate::tasks::tag),
        )
        .route(
            "/api/v1/tasks/{id}/tags/{tag_id}",
            axum::routing::delete(crate::tasks::untag),
        )
        // The vocabulary, as distinct from its use above. `tag.manage` authors
        // it; `task.update` applies it. Without a list nothing can render a
        // picker, which is why the write endpoint beside it was unreachable
        // from a browser for its whole life.
        .route(
            "/api/v1/tags",
            get(crate::tags::list).post(crate::tags::create),
        )
        // ADR-018 caps depth at 1, so this is a list and never a tree. A read
        // and only a read: `docs/03` says the rollup is displayed, never
        // enforced, and there is no verb here that could enforce one.
        .route(
            "/api/v1/tasks/{id}/subtasks",
            get(crate::tasks::subtasks_of),
        )
        // Milestones. Authored per project, read with the tasks they are about.
        // Closing one moves no task — see `crate::milestones`.
        .route(
            "/api/v1/projects/{id}/milestones",
            get(crate::milestones::list).post(crate::milestones::create),
        )
        .route(
            "/api/v1/milestones/{id}",
            axum::routing::patch(crate::milestones::update),
        )
        // C-015. Registered here with every other route — above the layers, so
        // it is wrapped by CSRF, the rate limiter and `observe` like anything
        // else. A stream that escaped those would be an unmetered, unlimited,
        // unidentified connection.
        .route("/api/v1/stream", get(crate::sse::stream))
        // C-021 — export. Registered here with every other route, above the
        // layers, for the reason this function's docs give.
        .route(
            "/api/v1/exports",
            axum::routing::post(crate::exports::create),
        )
        .route("/api/v1/exports/{id}", get(crate::exports::read))
        .route(
            "/api/v1/exports/{id}/download",
            get(crate::exports::download),
        )
        // C-002 — workspaces, membership, teams. Registered HERE, above the
        // layers, for the reason this function's docs give: a route appended to
        // the returned Router escapes the CSRF guard and the request id.
        .route(
            "/api/v1/workspaces",
            get(crate::workspaces::list).post(crate::workspaces::create),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}",
            get(crate::workspaces::read).patch(crate::workspaces::rename),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/members",
            get(crate::workspaces::list_members).post(crate::workspaces::add_member),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/members/{user_id}",
            axum::routing::delete(crate::workspaces::remove_member),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/invitations",
            get(crate::invitations::list).post(crate::invitations::create),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/invitations/{id}",
            axum::routing::delete(crate::invitations::revoke),
        )
        // Accepting is NOT under /workspaces: the acceptor may not be a member
        // of one yet, and may have no account at all. It sits beside the other
        // credential-bearing, unauthenticated endpoints instead.
        .route(
            "/api/v1/auth/invitations/accept",
            axum::routing::post(crate::invitations::accept),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/teams",
            get(crate::workspaces::list_teams).post(crate::workspaces::create_team),
        )
        .route(
            "/api/v1/teams/{team_id}/members",
            get(crate::workspaces::list_team_members).post(crate::workspaces::add_team_member),
        )
        .route(
            "/api/v1/teams/{team_id}/members/{user_id}",
            axum::routing::delete(crate::workspaces::remove_team_member),
        )
        // CSRF sits over every route, so a route added later cannot be added
        // beside it. docs/05: "every unsafe method without a valid token is
        // rejected" — every, not most.
        //
        // Under `observe`, so that a CSRF rejection still gets a request id and
        // still counts in the metrics. A refusal nobody can measure is a
        // refusal nobody notices.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::csrf_guard,
        ))
        // Outside CSRF, inside `observe`. Outside, because `docs/21`
        // §Enforcement order puts the cheapest checks first and a bucket check
        // is cheaper than an HMAC; inside, so a 429 still gets a request id and
        // still lands in the RED metrics — a refusal nobody can measure is a
        // refusal nobody notices.
        //
        // Its state is built here rather than added to `AppState`, so the
        // limiter's lifetime is the router's: every test gets its own, and no
        // other construction site of `AppState` has to change.
        .layer(axum::middleware::from_fn_with_state(
            crate::rate_limit::RateLimitState::auth(Arc::clone(&state.metrics)),
            crate::rate_limit::rate_limit,
        ))
        // The per-`(workspace, actor)` limiter, OUTSIDE the auth-class one so
        // it is the first bucket a request meets, and outside CSRF for the same
        // reason that one is: `docs/21` §Enforcement order runs the cheapest
        // check first.
        //
        // This is step 4 of that order. It authenticates once — step 3, "cheap:
        // one indexed read" — and puts the answer in the request extensions, so
        // the extractors below it do not repeat the query. Placed any lower it
        // would be limiting requests that had already cost a permission
        // resolution and a tenant read, which is the work it exists to prevent.
        .layer(axum::middleware::from_fn_with_state(
            crate::rate_limit::PrincipalState {
                pool: state.pool.clone(),
                limits: crate::rate_limit::PrincipalLimits::new(Arc::clone(&state.metrics)),
            },
            crate::rate_limit::principal_rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(state.clone(), observe))
        .with_state(state)
}
