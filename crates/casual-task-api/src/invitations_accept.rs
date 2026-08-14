/// `POST /api/v1/auth/invitations/accept`.
///
/// **Unauthenticated by design.** The invitee may have no account, which is the
/// point of inviting by email. The invitation token is the authority, and it is
/// checked the same way every other credential in this system is: a selector
/// finds the row in one indexed read, a constant-time comparison verifies the
/// secret.
///
/// # Errors
///
/// `401` for an unknown, expired, revoked or already-accepted token, `403` when
/// a signed-in caller's address does not match the invited one, or a database
/// failure.
pub async fn accept(
    State(state): State<AppState>,
    request_id: RequestId,
    headers: HeaderMap,
    ValidJson(body): ValidJson<AcceptInvitation>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;

    // Parsed before anything else: a malformed token must not reach a query as
    // a parameter, and it fails with the same 401 a wrong one does.
    let Ok((selector, verifier)) = casual_task_identity::credential::split(body.token.trim())
    else {
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    };

    let mut tx = unit::begin(&state, &request_id).await?;

    // Through the ADR-032 seam: unscoped by necessity, because the workspace is
    // not known until this returns it.
    let pending = repo::find_pending(tx.as_mut(), selector)
        .await
        .map_err(|error| internal(&error, "looking up an invitation", &request_id))?;
    let stored = repo::pending_verifier(tx.as_mut(), selector)
        .await
        .map_err(|error| internal(&error, "reading the invitation verifier", &request_id))?
        .unwrap_or_default();

    let Some(pending) =
        pending.filter(|_| casual_task_identity::credential::verify(verifier, &stored))
    else {
        // Unknown, expired, revoked, already accepted, or a forged verifier
        // against a real selector: one refusal, so none is distinguishable.
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    };

    // Who is accepting? A signed-in session, if there is one; otherwise the
    // address the invitation names.
    let signed_in = current_user(tx.as_mut(), &headers).await?;
    let user_id = match signed_in {
        Some((user_id, email)) => {
            // TIED TO THE ADDRESS (docs/40 §Invitations). An invitation is not
            // a bearer token for whoever holds the link — forwarding the email
            // must not hand membership to the wrong person.
            if !email.eq_ignore_ascii_case(&pending.email) {
                return Err(ApiError::denied(codes::NO_GRANT, &request_id));
            }
            user_id
        }
        None => {
            match repo::user_by_email(tx.as_mut(), &pending.email)
                .await
                .map_err(|error| internal(&error, "finding the invitee", &request_id))?
            {
                Some(existing) => existing,
                None => {
                    let display = body
                        .display_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| local_part(&pending.email));
                    repo::insert_user(tx.as_mut(), &pending.email, display)
                        .await
                        .map_err(|error| {
                            internal(&error, "creating the invited account", &request_id)
                        })?
                }
            }
        }
    };

    // Burn FIRST, inside the transaction. `consume_invitation` updates only a
    // row that is still pending, so two concurrent acceptances both find a live
    // invitation and exactly one proceeds — and the loser changes nothing
    // rather than both adding a membership and the second silently winning.
    let burned = repo::consume(tx.as_mut(), pending.id)
        .await
        .map_err(|error| internal(&error, "spending the invitation", &request_id))?;
    if !burned {
        return Ok(ApiError::unauthenticated(&request_id).into_response());
    }

    // The scope is minted here, before the membership exists, and made true by
    // this transaction — the same bootstrap `crate::workspaces::create` uses
    // for the creator's own membership. The invitation is the authority: it was
    // issued by someone already inside, and it has just been verified.
    let workspace = WorkspaceId::from_uuid(pending.workspace_id);
    let context = AuthContext::authenticated(
        casual_task_model::UserId::from_uuid(user_id),
        workspace,
        casual_task_model::ActorType::User,
    );
    let scope = context.scope();
    let mut scoped = Scoped::apply(&mut tx, &scope)
        .await
        .map_err(|error| internal(&error, "applying the tenant scope", &request_id))?;

    workspace_repo::insert_member(&mut scoped, user_id, "MEMBER")
        .await
        .map_err(|error| internal(&error, "adding the member", &request_id))?;

    if let Some(role_id) = pending.role_id {
        // `granted_by` is the INVITER, read back from the row. The audit
        // question is "who gave them this authority", and the answer is never
        // "they did" — recording the acceptor would read, years later, as a
        // self-grant. It falls back to the acceptor only if the inviter's
        // account has since been deleted, which `invited_by`'s nullable
        // foreign key permits.
        let inviter = repo::inviter_of(&mut scoped, pending.id)
            .await
            .map_err(|error| internal(&error, "reading the inviter", &request_id))?
            .unwrap_or(user_id);
        repo::assign_role(&mut scoped, user_id, role_id, inviter)
            .await
            .map_err(|error| internal(&error, "assigning the invited role", &request_id))?;
    }

    // docs/04 §Caching: bumped in the same transaction as the change, so a
    // stale permission-cache entry cannot be read — the key simply misses.
    workspace_repo::bump_authz_epoch(&mut scoped)
        .await
        .map_err(|error| internal(&error, "bumping authz_epoch", &request_id))?;

    let who = Provenance {
        actor: Some(casual_task_model::UserId::from_uuid(user_id)),
        actor_type: casual_task_model::ActorType::User,
        request_id: Uuid::parse_str(&request_id)
            .ok()
            .map(casual_task_model::RequestId::from_uuid),
        correlation_id: None,
        ip: crate::auth::client_ip(&headers),
        user_agent: headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned),
    };
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "workspace".to_owned(),
            aggregate_id: pending.workspace_id,
            project_id: None,
            event_type: "user.invitation.accepted".to_owned(),
            activity_changes: serde_json::json!({ "user_id": user_id }),
            audit_changes: serde_json::json!({
                "before": serde_json::Value::Null,
                "after": { "user_id": user_id, "role_id": pending.role_id },
            }),
            payload: serde_json::json!({
                "workspace_id": pending.workspace_id,
                "user_id": user_id,
                "invitation_id": pending.id,
            }),
            schema_version: SCHEMA_VERSION,
        },
        &who,
    )
    .await
    .map_err(|error| internal(&error, "recording the acceptance", &request_id))?;

    unit::commit(tx, &request_id).await?;

    // No session is issued. Accepting proves control of a mailbox, not of a
    // password — signing the caller in here would turn a forwarded email into
    // an authenticated session, which is the attack the address check above
    // exists to stop. The client sends them to sign in, or to set a password
    // through the reset flow if the account was just created.
    Ok((
        StatusCode::OK,
        axum::Json(AcceptedInvitation {
            workspace_id: pending.workspace_id,
            user_id,
        }),
    )
        .into_response())
}

/// Require `role.assign`, then control 1 of `docs/04`'s grant ceiling.
///
/// Split out so the two halves are visible as two rules rather than one
/// condition: the first is "may you grant roles at all", the second is "may you
/// grant *this* one".
async fn authorize_role_grant(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    role_id: Uuid,
    request_id: &str,
) -> Result<(), ApiError> {
    // Control 2 — the scope-appropriate assign permission. An invitation
    // carrying a role creates a WORKSPACE-scope grant on acceptance, and
    // `docs/04` names `role.assign` for exactly that.
    unit::authorized(
        ctx.authority.may_in_workspace(permission::ROLE_ASSIGN),
        request_id,
    )?;

    if !repo::role_exists(scoped, role_id)
        .await
        .map_err(|error| internal(&error, "checking the role", request_id))?
    {
        // 422, not 404: the id is well formed, it names nothing here. Refused
        // at invite time so a bad role is not discovered at acceptance time,
        // when the invitee would join with no role and nobody would know why.
        return Err(ApiError::unprocessable(
            codes::REFERENCE_NOT_FOUND,
            "No such role in this workspace",
            request_id,
        ));
    }

    let held = repo::role_permissions(scoped, role_id)
        .await
        .map_err(|error| internal(&error, "reading the role's permissions", request_id))?;

    // Control 1 — you cannot grant what you do not hold, checked permission by
    // permission so the refusal names the one that failed.
    for key in &held {
        // Against the CLOSED registry: a permission string in the database that
        // is not in `Permission::ALL` fails closed rather than being waved
        // through, because an unknown authority is one nobody has reasoned about.
        let Some(known) = permission::ALL.iter().find(|p| p.as_str() == key) else {
            tracing::error!(
                permission = key,
                "a role carries a permission not in the registry"
            );
            return Err(ApiError::denied(codes::NO_GRANT, request_id));
        };
        unit::authorized(ctx.authority.may_in_workspace(*known), request_id)?;
    }
    Ok(())
}

/// Hand the message to the relay, off the request path.
fn deliver(mailer: std::sync::Arc<dyn Mailer>, message: Message) {
    tokio::spawn(async move {
        if let Err(error) = mailer.send(&message).await {
            // `message` is safe to log: its Debug redacts the body, which is
            // the half that carries the token.
            tracing::error!(%error, ?message, "an invitation email was not delivered");
        }
    });
}

/// The email body. A link and nothing else sensitive.
///
/// No workspace name and no inviter name: this is delivered to an address
/// nobody has yet proved they control, so it must not reveal who is working
/// with whom. `docs/29` §Email content governs notification mail; the same
/// reasoning applies harder here.
#[must_use]
pub fn invite_body(public_url: &str, token: &str) -> String {
    format!(
        "You have been invited to a workspace on TaskForge.\n\
         \n\
         Open this link to accept:\n\
         {}{ACCEPT_PATH}?token={token}\n\
         \n\
         The link works once and expires in seven days. It is tied to this\n\
         email address and cannot be used with a different account.\n\
         \n\
         If you were not expecting this, you can ignore this message.\n",
        public_url.trim_end_matches('/')
    )
}

/// The signed-in user, if the request carries a live session.
///
/// Read directly rather than through the [`crate::middleware::Authenticated`]
/// extractor, because this endpoint must work **without** a credential and an
/// extractor that rejects cannot express "optional".
async fn current_user(
    conn: &mut sqlx::PgConnection,
    headers: &HeaderMap,
) -> Result<Option<(Uuid, String)>, ApiError> {
    let Some(selector) = crate::auth::session_selector(headers) else {
        return Ok(None);
    };
    let Some(session) = identity::live_session(conn, &selector)
        .await
        .map_err(|error| {
            tracing::error!(%error, "session lookup failed");
            ApiError::internal("invitation")
        })?
    else {
        return Ok(None);
    };
    let email = identity::email_of(conn, session.user_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the signed-in address failed");
            ApiError::internal("invitation")
        })?;
    Ok(email.map(|email| (session.user_id, email)))
}

/// The part before the `@`, as a default display name.
fn local_part(email: &str) -> &str {
    email.split('@').next().unwrap_or(email)
}

/// Reject an address this system cannot send to.
///
/// Deliberately minimal — one `@`, no whitespace, no control characters, and a
/// length bound. A full RFC 5322 validator rejects addresses that work, and the
/// real test of an address is whether mail reaches it. What this **must** catch
/// is the newline that would let an address carry its own headers; that is also
/// refused by `casual-task-infra`, and refusing it twice is cheaper than
/// deciding which layer owns it.
fn valid_email<'a>(email: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = email.trim();
    let refuse = || {
        ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "That is not an email address this system can send to",
            request_id,
        )
    };
    if trimmed.is_empty() || trimmed.len() > 320 {
        return Err(refuse());
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refuse());
    }
    let mut parts = trimmed.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(refuse());
    };
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err(refuse());
    }
    Ok(trimmed)
}

fn provenance_of(ctx: &Context) -> Provenance {
    ctx.provenance.clone()
}

fn body_of(record: &repo::InvitationRecord) -> InvitationBody {
    InvitationBody {
        id: record.id,
        email: record.email.clone(),
        role_id: record.role_id,
        invited_by: record.invited_by,
        expires_at: record
            .expires_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| String::new()),
        created_at: record
            .created_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| String::new()),
    }
}

fn internal(error: &sqlx::Error, doing: &str, request_id: &str) -> ApiError {
    tracing::error!(%error, doing, "invitation request failed");
    ApiError::internal(request_id)
}
