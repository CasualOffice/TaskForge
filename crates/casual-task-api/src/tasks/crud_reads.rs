/// `GET /api/v1/tasks/{id}` — 200 with an `ETag`, or 404.
///
/// # Errors
///
/// `404` when the task does not exist, is deleted, or sits in a project the
/// caller cannot see. All three are one answer (`docs/04`).
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

    let found = task::read_visible(&mut scoped, &ctx.viewer, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the task failed");
            ApiError::internal(&request_id)
        })?;
    unit::commit(tx, &request_id).await?;

    let Some((row, project_key)) = found else {
        return Err(ApiError::missing(codes::TASK_NOT_FOUND, &request_id));
    };
    Ok((
        StatusCode::OK,
        [(header::ETAG, etag::tag(row.version))],
        axum::Json(view(&row, &project_key)),
    )
        .into_response())
}

/// `GET /api/v1/tasks` — every task in the workspace the caller can reach.
///
/// # Errors
///
/// `400` for an unknown query parameter, a bad cursor, or an over-limit page.
pub async fn list(
    State(state): State<AppState>,
    member: WorkspaceMember,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::of_parts(&headers);
    let limit = wire::limit(
        params
            .get("limit")
            .map(|raw| {
                raw.parse::<u32>().map_err(|_| {
                    ApiError::bad_request(
                        codes::PAGE_TOO_LARGE,
                        "limit must be a number",
                        &request_id,
                    )
                })
            })
            .transpose()?,
        &request_id,
    )?;
    let after = wire::cursor(params.get("cursor").map(String::as_str), &request_id)?;

    // The whole grammar, read by `casual-task-search`. There is no second
    // parser here: `docs/27` §Compilation has one AST with two entry points,
    // and a handler that re-derived what `<` means would be the second one.
    //
    // `project_id` is accepted as an alias for the grammar's `project`. The
    // endpoint shipped with that spelling in C-006 and a name is a contract.
    let pairs: Vec<(&str, &str)> = params
        .iter()
        .map(|(name, value)| {
            let name = if name == "project_id" {
                "project"
            } else {
                name
            };
            (name, value.as_str())
        })
        .collect();
    let query = casual_task_search::parse_url(pairs).map_err(|error| {
        ApiError::bad_request(
            crate::error::Code::from_registry(error.code()),
            "The query could not be understood",
            &request_id,
        )
        .with_details(serde_json::json!({ "field": error.field() }))
    })?;

    if let Some(term) = full_text_term(&query.filter)
        && term.chars().count() > MAX_SEARCH_TERM
    {
        // docs/26 §Query limits: 256 characters. An unbounded term is an
        // unbounded `tsquery` construction.
        return Err(ApiError::bad_request(
            codes::SEARCH_TOO_LONG,
            "q must be at most 256 characters",
            &request_id,
        ));
    }

    let mut tx = unit::begin(&state, &request_id).await?;
    let mut scoped = unit::scope(&mut tx, &member, &request_id).await?;
    let ctx = Context::load_read(
        &state.metrics,
        &mut scoped,
        &member,
        &headers,
        &request_id,
        None,
    )
    .await?;

    // docs/04 §The list problem, step 1: resolved once, for the whole page.
    let accessible = project::accessible(&mut scoped, &ctx.viewer, MAX_ACCESSIBLE_PROJECTS)
        .await
        .map_err(|error| {
            tracing::error!(%error, "resolving the accessible project set failed");
            ApiError::internal(&request_id)
        })?;
    // The permission filter. A `project` clause the caller cannot see simply
    // intersects to nothing rather than returning a 404: it is a filter over a
    // list, and a list filtered to an invisible project is legitimately empty.
    let visible: Vec<ProjectId> = accessible
        .iter()
        .map(|(id, _)| ProjectId::from_uuid(*id))
        .collect();
    let keys: HashMap<Uuid, String> = accessible.into_iter().collect();

    // Symbols become values before validation, so `@me` never reaches the
    // compiler as the three characters a bind would send to the database.
    //
    // The offset is UTC, and that is a KNOWN GAP: `docs/27` §Timezone requires
    // the actor's, "`due before @today` must mean the same thing to someone in
    // Auckland and someone in Los Angeles", and `user_account` has nowhere to
    // store one. Recorded in `docs/14` rather than papered over with a header
    // nothing documents.
    let resolver = casual_task_search::Context::new(
        ctx.actor,
        ctx.viewer
            .teams
            .iter()
            .copied()
            .map(TeamId::from_uuid)
            .collect(),
        OffsetDateTime::now_utc(),
        crate::wire::caller_offset(&headers),
    );
    let filter = casual_task_search::resolve(&query.filter, &resolver).map_err(|error| {
        ApiError::bad_request(
            codes::UNKNOWN_SYMBOL,
            "The query uses a symbol this server does not know",
            &request_id,
        )
        .with_details(serde_json::json!({ "symbol": format!("{error:?}") }))
    })?;

    // Clause count and nesting depth (`docs/26` §Query limits). Checked after
    // resolution because resolution can rewrite a clause, and before
    // compilation because the compiler trusts its input.
    casual_task_search::validate(&filter).map_err(|error| {
        ApiError::bad_request(
            crate::error::Code::from_registry(error.code()),
            "The query exceeds a documented limit",
            &request_id,
        )
    })?;

    let sort = sort_for(&query, &filter, &request_id)?;
    let compiled = compile(
        &filter,
        ctx.workspace,
        &AuthorizedProjectSet::resolved(visible),
        &CompilerPage { sort, after, limit },
    );
    let mut rows = task::list(&mut scoped, &compiled).await.map_err(|error| {
        tracing::error!(%error, "listing tasks failed");
        ApiError::internal(&request_id)
    })?;

    // Who is on each task, for the whole page in one query, and inside the same
    // transaction as the page itself. Resolved per row this would be fifty
    // requests for a page of fifty — the difference between a list that can
    // show whose work this is and one that cannot afford to, and "what is mine"
    // is the question a list is opened with.
    //
    // The extra row the keyset fetches to answer "has more" is looked up too
    // and then dropped with it; one id is cheaper than a second round trip.
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let mut by_task: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (task_id, user_id) in task::assignees_for(&mut scoped, &ids)
        .await
        .map_err(|error| {
            tracing::error!(%error, "reading the page's assignees failed");
            ApiError::internal(&request_id)
        })?
    {
        by_task.entry(task_id).or_default().push(user_id);
    }
    unit::commit(tx, &request_id).await?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    // The cursor carries the key of the sort actually used, not always
    // `updated_at`. Carrying the wrong one resumes against a column the query
    // does not order by, which silently repeats or skips rows — and only ever
    // on the second page.
    let next_cursor = has_more
        .then(|| rows.last())
        .flatten()
        .map(|row| Cursor::new(vec![cursor_key(row, sort.field)], row.id).encode());

    let data: Vec<TaskView> = rows
        .iter()
        .map(|row| {
            let key = keys.get(&row.project_id).map_or("", String::as_str);
            let mut built = view(row, key);
            built.assignees = by_task.remove(&row.id).unwrap_or_default();
            built
        })
        .collect();

    Ok(axum::Json(Paged {
        data,
        page: Page {
            next_cursor,
            has_more,
        },
    })
    .into_response())
}

/// `docs/26` §Query limits: search term length 256 characters.
const MAX_SEARCH_TERM: usize = 256;
