//! Who may subscribe to what.
//!
//! # The failure this file exists to prevent
//!
//! A subscriber receiving an event for a task they could not have read through
//! `GET`. This is the widest-blast-radius leak in the product: a request-scoped
//! authorization mistake leaks one response to one caller, and a stream-scoped
//! one leaks every event on a project to a listener for as long as they stay
//! connected.
//!
//! So the check here is not "is this actor plausibly allowed" but the literal
//! question `GET` asks, followed by one more that `GET` does not have to ask.
//!
//! # The extra question, and why a stream needs it
//!
//! `GET /tasks/{id}` evaluates the actor's permission **against the task in
//! front of it** — its assignees, its reporter, its environment. A grant
//! constrained to `assignee_is_actor` is a perfectly good read permission; it
//! just answers differently for each task.
//!
//! A stream has no task in front of it. It is authorized once and then delivers
//! whatever arrives, and an `outbox_event` does not carry assignees or an
//! environment (migration 0022 added the project and stopped there, deliberately
//! — see its comment). So a constrained reader cannot be filtered per event
//! without facts nothing has.
//!
//! The resolution is to refuse rather than to guess: [`may_subscribe`] admits an
//! actor only if they may read **every** task in the project, whatever its
//! facts. That is asked directly — the permission is evaluated against the
//! worst-case task — so a new constraint added to `casual-task-authz` tomorrow
//! is handled correctly here today, with no list of constraint kinds to keep in
//! sync.
//!
//! **The cost, stated:** an actor whose read is constrained gets `403` on the
//! stream and has to poll. They are a minority of a minority — a constrained
//! grant is an unusual configuration — and the alternative is a filter that is
//! wrong in exactly the direction nobody notices.

use casual_task_app::ResourceFacts;
use casual_task_model::{ProjectId, permission};
use casual_task_persistence::Scoped;
use casual_task_persistence::project::{self, ProjectRow};
use uuid::Uuid;

use crate::context::Context;
use crate::error::{ApiError, codes};

/// Why a subscription was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The project does not exist, or the actor cannot see it. Not
    /// distinguished — `docs/04`: an absent resource and an invisible one
    /// return the same thing.
    NotVisible,
    /// The actor can see the project but holds no `task.read` there.
    NoReadPermission,
    /// The actor's read is constrained per task, so no per-event filter can be
    /// applied. See the module docs.
    ConstrainedReader,
}

impl Refusal {
    /// The HTTP answer.
    ///
    /// The two 403s carry **different** codes, which is `docs/20`'s ruling
    /// rather than this module's preference: `TF-AZN-0001` and `TF-AZN-0002` are
    /// distinct on purpose, because "you were never granted this" and "your
    /// grant did not apply here" have different fixes. Collapsing them would
    /// send an operator to re-grant a permission the actor already holds.
    #[must_use]
    pub fn into_error(self, request_id: &str) -> ApiError {
        match self {
            Self::NotVisible => ApiError::missing(codes::PROJECT_NOT_FOUND, request_id),
            Self::NoReadPermission => ApiError::forbidden(codes::NO_GRANT, request_id),
            Self::ConstrainedReader => {
                ApiError::forbidden(codes::CONSTRAINT_UNSATISFIED, request_id)
            }
        }
    }
}

/// May `ctx` subscribe to live events for `project`?
///
/// # Errors
///
/// A [`Refusal`] when the answer is no, or `500` when the visibility read
/// fails — a database error must not be reported as a permission answer.
pub async fn may_subscribe(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    project: Uuid,
    request_id: &str,
) -> Result<Result<ProjectRow, Refusal>, ApiError> {
    // Exactly what `GET /projects/{id}` calls. Not a re-implementation of the
    // visibility rule: the same function, so the two cannot drift apart. A
    // second copy of `VISIBLE` is how a stream ends up more generous than the
    // endpoint it mirrors.
    let Some(row) = project::read_visible(scoped, &ctx.viewer, project)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project visibility for a stream failed");
            ApiError::internal(request_id)
        })?
    else {
        return Ok(Err(Refusal::NotVisible));
    };

    let member = project::is_member(scoped, project, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership for a stream failed");
            ApiError::internal(request_id)
        })?;
    let team = row.teams();

    // Question one: may they read tasks here at all, on the most favourable
    // reading of the facts? If not, there is nothing to discuss.
    let best_case = ResourceFacts {
        assignees: vec![ctx.actor],
        reporter: Some(ctx.actor),
        actor_is_project_member: member,
        environment: None,
        actor_is_guest: ctx.is_guest,
    };
    if !ctx
        .authority
        .may_in_project(
            permission::TASK_READ,
            ProjectId::from_uuid(project),
            &team,
            &best_case,
        )
        .is_allowed()
    {
        return Ok(Err(Refusal::NoReadPermission));
    }

    // Question two, the one a stream has to ask. The worst-case task: assigned
    // to nobody, reported by someone else, tagged to no environment. Only the
    // two facts that are about the *actor* rather than the task — project
    // membership and guest standing — keep their real values, because those do
    // not vary from task to task.
    //
    // If the permission survives that, it survives every task in the project,
    // and a subscriber cannot be sent something a `GET` would have refused.
    let worst_case = ResourceFacts {
        assignees: Vec::new(),
        reporter: None,
        actor_is_project_member: member,
        environment: None,
        actor_is_guest: ctx.is_guest,
    };
    if !ctx
        .authority
        .may_in_project(
            permission::TASK_READ,
            ProjectId::from_uuid(project),
            &team,
            &worst_case,
        )
        .is_allowed()
    {
        return Ok(Err(Refusal::ConstrainedReader));
    }

    Ok(Ok(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_narrow_refusals_are_both_403_and_say_which() {
        // docs/20 keeps TF-AZN-0001 and -0002 distinct because the fixes
        // differ: one needs a grant, the other needs a constraint relaxed.
        let no_permission = Refusal::NoReadPermission.into_error("r");
        let constrained = Refusal::ConstrainedReader.into_error("r");
        assert_eq!(no_permission.status(), axum::http::StatusCode::FORBIDDEN);
        assert_eq!(constrained.status(), axum::http::StatusCode::FORBIDDEN);
        assert_ne!(
            no_permission.code().as_str(),
            constrained.code().as_str(),
            "the two refusals are indistinguishable, so an operator cannot tell \
             a missing grant from a constraint that did not apply"
        );
    }

    #[test]
    fn an_invisible_project_is_a_404_not_a_403() {
        // docs/04: absent and invisible return the same thing. A 403 here would
        // confirm the project exists to someone who cannot see it.
        assert_eq!(
            Refusal::NotVisible.into_error("r").status(),
            axum::http::StatusCode::NOT_FOUND
        );
    }
}
