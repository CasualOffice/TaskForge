/// `PATCH /api/v1/tasks/{id}` — update plain fields.
///
/// # Errors
///
/// `400` for a malformed body or an attempt to write `status`, `404` when the
/// task is not visible, `409` against a stale version, `428` without
/// `If-Match`, `403` without `task.update`.
pub async fn update(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Body(body): Body<PatchRequest>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    // Before anything is read: a client that forgot the header has a bug, and
    // the answer does not depend on whether the task exists.
    let expected = etag::if_match(&headers, &request_id)?;

    if body.status_id.is_some() || body.state.is_some() {
        return Err(ApiError::bad_request(
            codes::STATUS_NOT_DIRECTLY_WRITABLE,
            "Status is never written directly — POST to /tasks/{id}/transitions, \
             which is what enforces transition validity, required fields, \
             dependency gating and the transition's own permission",
            &request_id,
        ));
    }

    let title = body
        .title
        .as_deref()
        .map(|t| validated_title(t, &request_id))
        .transpose()?;
    let task_type = body
        .task_type
        .as_deref()
        .map(|v| one_of(Some(v), TASK_TYPES, "TASK", "type", &request_id))
        .transpose()?;
    let priority = body
        .priority
        .as_deref()
        .map(|v| one_of(Some(v), PRIORITIES, "NONE", "priority", &request_id))
        .transpose()?;
    if let Some(Some(description)) = body.description.as_ref()
        && description.len() > 65_536
    {
        return Err(ApiError::bad_request(
            codes::OUT_OF_RANGE,
            "description must be at most 65536 bytes",
            &request_id,
        ));
    }
    let start_at = optional_timestamp(body.start_at.as_ref(), "start_at", &request_id)?;
    let due_at = optional_timestamp(body.due_at.as_ref(), "due_at", &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    // docs/23 §Validation order: readable (404), version (409), permission
    // (403). The version check precedes the permission check deliberately — the
    // actor can already see the task, so its version is not a secret, and the
    // stale-client case is overwhelmingly the common one.
    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    if current.version != expected {
        return Err(conflict(&current, &project_key, expected, &request_id));
    }
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_UPDATE,
        &request_id,
    )
    .await?;

    // Changing the type is raising the new one, and it has to be authorized as
    // such. Without this, a grant narrowed to `task_type_in [BUG]` is escaped
    // in two requests: raise a bug, then convert it to a feature. The check is
    // against the *new* type, on top of the update check already made against
    // the old — both must hold, because the actor is asserting authority over
    // the task as it will be as well as as it is.
    if let Some(wanted) = task_type
        && wanted != current.task_type
    {
        let is_member = project::is_member(&mut scoped, current.project_id, ctx.actor.as_uuid())
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading project membership failed");
                ApiError::internal(&request_id)
            })?;
        let project_row = project::read_visible(&mut scoped, &ctx.viewer, current.project_id)
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading the project failed");
                ApiError::internal(&request_id)
            })?
            .ok_or_else(|| ApiError::missing(codes::PROJECT_NOT_FOUND, &request_id))?;
        unit::authorized(
            ctx.authority.may_in_project(
                permission::TASK_CREATE,
                ProjectId::from_uuid(current.project_id),
                &project_row.teams(),
                &casual_task_app::ResourceFacts {
                    task_type: super::guard::task_type_of(wanted),
                    ..ctx.facts_in_project(is_member)
                },
            ),
            &request_id,
        )?;
    }

    let patch = task::TaskPatch {
        title: title.map(ToOwned::to_owned),
        description: body.description.clone(),
        task_type: task_type.map(ToOwned::to_owned),
        priority: priority.map(ToOwned::to_owned),
        start_at,
        due_at,
    };
    let updated = task::update(
        &mut scoped,
        current.id,
        expected,
        &patch,
        ctx.actor.as_uuid(),
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "updating the task failed");
        ApiError::internal(&request_id)
    })?;

    // Zero rows means someone committed between the read above and this
    // statement. docs/24: "0 rows affected ⇒ someone else wrote first ⇒ 409".
    let Some(updated) = updated else {
        let (now, key) = visible(&mut scoped, &ctx, id, &request_id).await?;
        return Err(conflict(&now, &key, expected, &request_id));
    };

    let before = serde_json::json!(view(&current, &project_key));
    let after_view = view(&updated, &project_key);
    let after = serde_json::json!(after_view);
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: updated.id,
            project_id: Some(updated.project_id),
            event_type: "task.updated".to_owned(),
            activity_changes: changed_fields(&current, &updated),
            audit_changes: serde_json::json!({ "before": before, "after": after }),
            payload: after.clone(),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the task update failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(updated.version))],
        axum::Json(after_view),
    )
        .into_response())
}

/// `DELETE /api/v1/tasks/{id}` — soft delete.
///
/// # Errors
///
/// `404` when the task is not visible, `409` against a stale version, `428`
/// without `If-Match`, `403` without `task.delete`.
pub async fn delete(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let expected = etag::if_match(&headers, &request_id)?;

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load(&state.metrics, &mut scoped, &member, &headers, &request_id).await?;

    let (current, project_key) = visible(&mut scoped, &ctx, id, &request_id).await?;
    if current.version != expected {
        return Err(conflict(&current, &project_key, expected, &request_id));
    }
    authorize_on_task(
        &mut scoped,
        &ctx,
        &current,
        permission::TASK_DELETE,
        &request_id,
    )
    .await?;

    let deleted = task::soft_delete(&mut scoped, current.id, expected, ctx.actor.as_uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "deleting the task failed");
            ApiError::internal(&request_id)
        })?;
    let Some(deleted) = deleted else {
        let (now, key) = visible(&mut scoped, &ctx, id, &request_id).await?;
        return Err(conflict(&now, &key, expected, &request_id));
    };

    let before = serde_json::json!(view(&current, &project_key));
    UnitOfWork::record(
        &mut scoped,
        &Change {
            aggregate_type: "task".to_owned(),
            aggregate_id: deleted.id,
            project_id: Some(deleted.project_id),
            event_type: "task.deleted".to_owned(),
            activity_changes: serde_json::json!({
                "key": format!("{project_key}-{}", deleted.number),
                "title": deleted.title,
            }),
            audit_changes: serde_json::json!({ "before": before, "after": null }),
            payload: serde_json::json!({
                "id": deleted.id,
                "project_id": deleted.project_id,
                "key": format!("{project_key}-{}", deleted.number),
            }),
            schema_version: 1,
        },
        &ctx.provenance,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "recording the task delete failed");
        ApiError::internal(&request_id)
    })?;
    unit::commit(tx, &request_id).await?;

    // 204: the representation is gone, and echoing a tombstone would invite a
    // client to render it.
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------

/// The full-text term in a filter, if it has one.
pub(crate) fn full_text_term(node: &Node) -> Option<&str> {
    match node {
        Node::Clause(clause) if clause.field == Field::Q => match &clause.value {
            Value::Literal(term) | Value::Symbol(term) => Some(term.as_str()),
            _ => None,
        },
        Node::And(children) | Node::Or(children) => children.iter().find_map(full_text_term),
        Node::Not(inner) => full_text_term(inner),
        Node::Clause(_) => None,
    }
}

/// Decide the ordering, refusing the two combinations that cannot work.
///
/// - **`rank` without `q`** — there is nothing to rank against. Silently
///   falling back to `updated_at` would answer a different question than the
///   one asked.
/// - **More than one sort key** — the cursor carries one key plus the id
///   tiebreaker, and the compiler emits one keyset comparison. Accepting
///   `sort=-due_at,key` and honouring only the first would produce a
///   non-deterministic order across pages, which is the bug the mandatory
///   tiebreaker exists to prevent. `docs/27` documents the multi-key form, so
///   this is a **gap reported as an error**, not a silent truncation.
pub(crate) fn sort_for(
    query: &casual_task_search::Query,
    filter: &Node,
    request_id: &str,
) -> Result<Sort, ApiError> {
    let searching = full_text_term(filter).is_some();
    if query.sorts.len() > 1 {
        return Err(ApiError::bad_request(
            codes::UNSORTABLE_FIELD,
            "Only one sort key is supported; a second would make the cursor \
             non-deterministic",
            request_id,
        ));
    }
    let Some(sort) = query.sorts.first().copied() else {
        // A search with no explicit order is ordered by relevance — the only
        // ordering a search box implies.
        return Ok(if searching {
            Sort {
                field: SortField::Rank,
                direction: Direction::Desc,
            }
        } else {
            Sort::default()
        });
    };
    if sort.field == SortField::Rank && !searching {
        return Err(ApiError::bad_request(
            codes::UNSORTABLE_FIELD,
            "sort=rank requires a q parameter — there is nothing to rank without one",
            request_id,
        ));
    }
    Ok(sort)
}

/// The value a cursor must carry to resume the given sort.
///
/// Must agree with the compiler's `cursor_type` for the same field: the
/// parameter is cast to that type on the way back in, so a value formatted
/// differently here fails at execution on page two.
pub(crate) fn cursor_key(row: &TaskRow, field: SortField) -> String {
    match field {
        SortField::CreatedAt => wire::timestamp(row.created_at),
        SortField::UpdatedAt => wire::timestamp(row.updated_at),
        SortField::DueAt => row.due_at.map(wire::timestamp).unwrap_or_default(),
        SortField::Priority => row.priority.clone(),
        SortField::Position => row.position.clone(),
        SortField::Key => row.number.to_string(),
        SortField::Rank => row.rank.unwrap_or_default().to_string(),
        // Not reachable: the compiler's `ws.position` has no FROM entry, so a
        // status-position sort cannot execute and is refused before here.
        SortField::StatusPosition => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Update, delete
// ---------------------------------------------------------------------------

/// `PATCH /api/v1/tasks/{id}`.
///
/// `status_id` and `state` are **accepted and then refused** with
/// `TF-WFL-0001`. Leaving them out of the struct would make them unknown fields
/// — a `400` saying "we have never heard of `status_id`", when the truth is
/// that the field exists and has its own door (`docs/23` §The transition
/// command). The same argument `docs/23` makes for why the door exists at all
/// is the reason the error has to say so.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    #[serde(default)]
    pub title: Option<String>,
    /// `Option<Option<_>>`: absent leaves it alone, `null` clears it
    /// (`docs/05` §Conventions).
    #[serde(default, deserialize_with = "wire::double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, rename = "type")]
    pub task_type: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default, deserialize_with = "wire::double_option")]
    pub start_at: Option<Option<String>>,
    #[serde(default, deserialize_with = "wire::double_option")]
    pub due_at: Option<Option<String>>,
    #[serde(default)]
    pub status_id: Option<Uuid>,
    #[serde(default)]
    pub state: Option<String>,
}
