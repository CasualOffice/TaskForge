/// `GET /api/v1/projects/{id}` — 200 with an `ETag`, or 404.
///
/// # Errors
///
/// `404` when the project does not exist **or** the caller cannot see it. The
/// two are indistinguishable by construction: the query returns no row for
/// either (`docs/04`).
pub async fn read(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let row = visible(&mut scoped, &ctx, id, &request_id).await?;
    unit::commit(tx, &request_id).await?;

    Ok(representation(&row))
}

/// `PATCH /api/v1/projects/{id}` — requires `If-Match`.
///
/// # Errors
///
/// `428` without `If-Match`, `409` when the version has moved, `404` when the
/// project is invisible, `403` without `project.update`, `422` for an attempt
/// to change the key.
pub async fn update(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Body(body): Body<PatchRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    // Before anything is read: a client that forgot the header has a bug, and
    // the answer does not depend on whether the project exists.
    let expected = etag::if_match(&headers, &request_id)?;

    if body.key.is_some() {
        return Err(ApiError::unprocessable(
            codes::PROJECT_KEY_IMMUTABLE,
            "A project key cannot be changed (ADR-007): task keys appear in \
             commits, chat, and external tickets",
            &request_id,
        ));
    }
    let name = body
        .name
        .as_deref()
        .map(|name| validated_name(name, &request_id))
        .transpose()?;
    let visibility = body
        .visibility
        .as_deref()
        .map(|v| validated_visibility(Some(v), &request_id))
        .transpose()?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // docs/23 §Validation order fixes this sequence: readable (404), version
    // (409), permission (403). The version check precedes the permission check
    // deliberately — the actor can already see the project, so its version is
    // not a secret, and reporting the conflict first is the more actionable
    // error for the overwhelmingly common case of a stale client.
    let current = visible(&mut scoped, &ctx, id, &request_id).await?;
    if current.version != expected {
        return Err(conflict(&current, expected, &request_id));
    }
    let is_member = project::is_member(&mut scoped, current.id, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading project membership failed");
            ApiError::internal(&request_id)
        })?;
    unit::authorized(
        ctx.authority.may_in_project(
            permission::PROJECT_UPDATE,
            ProjectId::from_uuid(current.id),
            &current.teams(),
            &ctx.facts_in_project(is_member),
        ),
        &request_id,
    )?;

    let patch = ProjectPatch {
        name: name.map(ToOwned::to_owned),
        description: body.description.clone(),
        visibility: visibility.map(ToOwned::to_owned),
    };
    let updated = project::update(
        &mut scoped,
        current.id,
        expected,
        &patch,
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "updating the project failed");
        ApiError::internal(&request_id)
    })?;

    // Zero rows means someone committed between the read above and this
    // statement. docs/24: "0 rows affected ⇒ someone else wrote first ⇒ 409".
    let Some(updated) = updated else {
        let now = visible(&mut scoped, &ctx, id, &request_id).await?;
        return Err(conflict(&now, expected, &request_id));
    };

    let after = serde_json::json!(ProjectView::from(&updated));
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "project".to_owned(),
            aggregate_id: updated.id,
            project_id: Some(updated.id),
            event_type: "project.updated".to_owned(),
            activity_changes: serde_json::json!({ "name": updated.name }),
            audit_changes: serde_json::json!({
                "before": serde_json::json!(ProjectView::from(&current)),
                "after": after,
            }),
            payload: after.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the project update failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok(representation(&updated))
}

/// 200 with the `ETag` the next `If-Match` must carry.
fn representation(row: &ProjectRow) -> Response {
    (
        StatusCode::OK,
        [(header::ETAG, etag::tag(row.version))],
        axum::Json(ProjectView::from(row)),
    )
        .into_response()
}

/// The `409` body `docs/24` §The conflict response describes.
///
/// `conflicting_fields` and `your_safe_fields` are **not** here: producing them
/// needs the pre-image the caller was editing, which no request carries and no
/// table stores yet. They are named in `docs/14` rather than approximated —
/// a wrong "these fields are safe to retry" is worse than an absent one,
/// because the client acts on it automatically.
fn conflict(current: &ProjectRow, your_version: i64, request_id: &str) -> ApiError {
    ApiError::conflict(
        codes::VERSION_CONFLICT,
        "This project was updated by someone else",
        request_id,
    )
    .with_details(serde_json::json!({
        "your_version": your_version,
        "current_version": current.version,
        "changed_by": current.updated_by,
        "changed_at": wire::timestamp(current.updated_at),
        "current": ProjectView::from(current),
    }))
}

/// Read a project the caller may see, or refuse with `404`.
async fn visible(
    scoped: &mut Scoped<'_>,
    ctx: &Context,
    id: Uuid,
    request_id: &str,
) -> Result<ProjectRow, ApiError> {
    project::read_visible(scoped, &ctx.viewer, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the project failed");
            ApiError::internal(request_id)
        })?
        .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, request_id))
}

fn validated_name<'a>(name: &'a str, request_id: &str) -> Result<&'a str, ApiError> {
    let trimmed = name.trim();
    // docs/21 bounds every input. The schema does not constrain project.name,
    // so the bound is here — an unbounded text field is a storage amplifier.
    if trimmed.is_empty() || trimmed.chars().count() > 200 {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "name must be between 1 and 200 characters",
            request_id,
        ));
    }
    Ok(trimmed)
}

fn validated_visibility<'a>(value: Option<&'a str>, request_id: &str) -> Result<&'a str, ApiError> {
    // The database default is TEAM (migration 0004); the API's default matches
    // it rather than choosing its own, so a project created through either has
    // the same visibility.
    let Some(value) = value else {
        return Ok("TEAM");
    };
    if VISIBILITIES.contains(&value) {
        Ok(value)
    } else {
        Err(ApiError::bad_request(
            codes::INVALID_ENUM,
            "visibility must be PRIVATE, TEAM, or WORKSPACE",
            request_id,
        )
        .with_details(serde_json::json!({ "allowed": VISIBILITIES })))
    }
}

/// `limit` and `cursor`, validated.
fn page_params(
    params: &HashMap<String, String>,
    request_id: &str,
) -> Result<(u32, Option<(OffsetDateTime, Uuid)>), ApiError> {
    unit::reject_unknown(params, &["limit", "cursor"], request_id)?;
    let limit = wire::limit(
        params
            .get("limit")
            .map(|raw| {
                raw.parse::<u32>().map_err(|_| {
                    ApiError::bad_request(
                        codes::PAGE_TOO_LARGE,
                        "limit must be a number",
                        request_id,
                    )
                })
            })
            .transpose()?,
        request_id,
    )?;
    let after = wire::cursor(params.get("cursor").map(String::as_str), request_id)?
        .map(|c| {
            let key = c.keys.first().cloned().unwrap_or_default();
            OffsetDateTime::parse(&key, &Rfc3339)
                .map(|at| (at, c.id))
                .map_err(|_| {
                    ApiError::bad_request(
                        codes::BAD_CURSOR,
                        "Malformed pagination cursor",
                        request_id,
                    )
                })
        })
        .transpose()?;
    Ok((limit, after))
}
