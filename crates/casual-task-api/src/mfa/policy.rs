//! Whether a workspace demands MFA, and whether a session satisfies it.
//!
//! # One place, because the alternative is two slightly different answers
//!
//! `AGENTS.md` §Module size and shape: the `guard` split "exists so 'may this
//! actor do this' cannot be assembled two different ways in two handlers, which
//! is how one endpoint ends up more permissive than the one beside it."
//! [`step_up_required`] is that single answer for MFA. Workspace resolution is
//! its only consumer today; when re-authentication for sensitive actions
//! arrives (`docs/40` §MFA lists five), it consumes this rather than
//! reimplementing it.
//!
//! # Why the check is here and not at login
//!
//! `docs/40` §Workspace-level SSO and MFA step-up: the browser session is
//! **user**-scoped — `user_account` is the only table without a `workspace_id`,
//! because a person spans workspaces — while MFA enforcement is **per
//! workspace**. A login therefore has no single policy to apply. The cost, which
//! `docs/40` states, is that "signed in" and "may enter this workspace" are two
//! questions and every workspace-scoped entry point must ask the second.
//!
//! # 401, not 403
//!
//! `docs/20` registers `TF-AUT-0005` "MFA required" as a **401**, and that is
//! the right shape: this is a statement about the *credential* being
//! incomplete, not about the actor lacking authority — the same distinction the
//! registry draws for `TF-AUT-0013`. A 403 would tell a client to give up; a
//! 401 tells it to strengthen the credential, which is exactly what a step-up
//! is.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use casual_task_persistence::{Change, Provenance, UnitOfWork, mfa as repo};
use time::OffsetDateTime;

use crate::context::Context;
use crate::error::{ApiError, codes};
use crate::json::ValidJson;
use crate::middleware::WorkspaceMember;
use crate::server::{AppState, RequestId};
use crate::unit;

use super::wire::SetRequirement;

/// The event schema version this module emits.
const SCHEMA_VERSION: i32 = 1;

/// Whether this session must step up before entering this workspace.
///
/// The **only** definition of "satisfied" in the codebase. It is deliberately
/// total and takes plain values rather than a request, so it can be tested
/// exhaustively without a database or a router.
///
/// # What counts as satisfied, and what does not
///
/// A session satisfies MFA when it carries an `mfa_satisfied_at` instant. **No
/// expiry is applied**, because `docs/40` states none — it says a workspace
/// "demanding more than the session carries triggers a step-up" and stops
/// there. Choosing a lifetime here would be settling a design question in an
/// implementation, which `AGENTS.md` forbids; the instant is nevertheless
/// recorded so that a lifetime can be added later without a migration, a client
/// change, or a second call site. Tracked as **D-064**.
#[must_use]
pub fn step_up_required(
    workspace_requires_mfa: bool,
    session_satisfied_at: Option<OffsetDateTime>,
) -> bool {
    workspace_requires_mfa && session_satisfied_at.is_none()
}

/// The refusal a caller who must step up receives.
///
/// Built here rather than at the call site so every step-up refusal carries the
/// same code and the same machine-readable hint. A client that cannot tell this
/// 401 from an expired-session 401 logs the user out instead of prompting for a
/// code, which is a worse outcome than either.
#[must_use]
pub fn step_up_refusal(request_id: &str) -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        codes::MFA_REQUIRED,
        "This workspace requires multi-factor authentication",
        request_id,
    )
    .with_details(serde_json::json!({
        "step_up": "/api/v1/auth/mfa/step-up",
    }))
}

/// `PUT /api/v1/workspaces/{workspace_id}/mfa-requirement`.
///
/// # The enforcing admin must already have MFA enrolled
///
/// `docs/40` §MFA states the rule and the reason: "the enforcing admin must
/// already have MFA enrolled, so nobody can lock themselves out while locking
/// others in." Without it, the first person to turn this on is locked out of
/// the workspace they administer by the setting they just saved, and the only
/// way back is the break-glass path — which is meant to be the last resort, not
/// the ordinary consequence of using a feature.
///
/// Turning enforcement **off** carries no such requirement: it can only widen
/// access, and demanding a factor from someone trying to remove the requirement
/// would be the lockout again wearing the opposite sign.
///
/// # Errors
///
/// `401` when enabling without a confirmed factor, `403` when the caller may
/// not manage the workspace, or a database failure.
pub async fn set_requirement(
    State(state): State<AppState>,
    member: WorkspaceMember,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<SetRequirement>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&mut scoped, &member, &headers, &request_id).await?;

    // Changing an authentication policy for everyone in the workspace is
    // workspace administration, which `docs/04` gives `workspace.manage`. This
    // is not the D-054 question: that one is about membership, and this
    // permission exists in the closed registry and means exactly this.
    unit::authorized(
        ctx.authority
            .may_in_workspace(casual_task_model::permission::WORKSPACE_MANAGE),
        &request_id,
    )?;

    if body.required {
        let enrolled = repo::has_confirmed_factor(scoped.conn(), ctx.actor.as_uuid())
            .await
            .map_err(|error| internal(&error, "checking the admin's own factor", &request_id))?;
        if !enrolled {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                codes::MFA_REQUIRED,
                "Enrol in multi-factor authentication before requiring it of others",
                &request_id,
            )
            .with_details(serde_json::json!({
                "enrol": "/api/v1/auth/mfa/enrolment",
            })));
        }
    }

    repo::set_workspace_mfa(&mut scoped, body.required)
        .await
        .map_err(|error| internal(&error, "setting the MFA requirement", &request_id))?;

    // docs/04 §Caching: bumped in the same transaction as the change, so a
    // stale permission-cache entry cannot be read.
    casual_task_persistence::workspace::bump_authz_epoch(&mut scoped)
        .await
        .map_err(|error| internal(&error, "bumping authz_epoch", &request_id))?;

    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: member.context.scope().id().as_uuid(),
            project_id: None,
            event_type: "workspace.mfa.requirement.changed".to_owned(),
            activity_changes: serde_json::json!({ "require_mfa": body.required }),
            audit_changes: serde_json::json!({
                "before": { "require_mfa": !body.required },
                "after": { "require_mfa": body.required },
            }),
            payload: serde_json::json!({
                "workspace_id": member.context.scope().id().as_uuid(),
                "require_mfa": body.required,
            }),
            schema_version: SCHEMA_VERSION,
        },
        &provenance(&ctx),
    )
    .await
    .map_err(|error| internal(&error, "recording the policy change", &request_id))?;

    unit::commit(tx, &request_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn provenance(ctx: &Context) -> Provenance {
    ctx.provenance.clone()
}

pub(crate) fn internal(error: &sqlx::Error, doing: &str, request_id: &str) -> ApiError {
    tracing::error!(%error, doing, "an MFA request failed");
    ApiError::internal(request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn an_instant() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_760_000_000).expect("valid")
    }

    #[test]
    fn a_workspace_that_does_not_require_mfa_never_demands_a_step_up() {
        // The common case, and the one a regression would break loudest.
        assert!(!step_up_required(false, None));
        assert!(!step_up_required(false, Some(an_instant())));
    }

    #[test]
    fn a_requiring_workspace_demands_a_step_up_from_an_unsatisfied_session() {
        assert!(step_up_required(true, None));
    }

    #[test]
    fn a_satisfied_session_enters_a_requiring_workspace() {
        // The companion: a guard that always demanded a step-up would satisfy
        // the test above and make every requiring workspace unenterable.
        assert!(!step_up_required(true, Some(an_instant())));
    }

    #[test]
    fn the_refusal_names_where_to_go_next() {
        // A 401 a client cannot distinguish from an expired session makes it
        // log the user out instead of prompting for a code.
        let error = step_up_refusal("r");
        assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(error.code().as_str(), "TF-AUT-0005");
    }
}
