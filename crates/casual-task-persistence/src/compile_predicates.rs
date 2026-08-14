/// The single full-text term in a filter, if it has one.
///
/// Only the first is honoured. Two `q` clauses would need two ranks, while the
/// URL form represents a repeated parameter as one value.
fn full_text_term(node: &Node) -> Option<String> {
    match node {
        Node::Clause(clause) if clause.field == Field::Q => match &clause.value {
            Value::Literal(term) | Value::Symbol(term) => Some(term.clone()),
            _ => None,
        },
        Node::And(children) | Node::Or(children) => children.iter().find_map(full_text_term),
        Node::Not(inner) => full_text_term(inner),
        Node::Clause(_) => None,
    }
}

/// The ranked full-text query (`docs/26` §Permission filtering).
///
/// # The tenant predicate is on the projection, not only on `task`
///
/// `s.workspace_id = $1 AND s.project_id = ANY($2)` are applied to
/// `task_search` itself so `task_search_scope_ix` can serve them and be
/// combined with `task_search_gin` in a `BitmapAnd`. Filtering only through the
/// join to `task` would leave the planner scanning the whole projection and
/// discarding rows afterwards — which is the shape **D-043** measured as a
/// `Parallel Seq Scan` at reference scale.
///
/// **D-043 is not closed by this.** Under row-level security `@@` resolves to
/// `ts_match_vq`, which is not `LEAKPROOF`, so PostgreSQL will not evaluate it
/// before the row-security qual and cannot use the GIN index as an index qual
/// at all. This shape is the "tenant-filtered projection" that decision says to
/// try first; whether it is enough is a measurement, and the answer is recorded
/// in `docs/14` §D-043 rather than assumed here.
fn compile_search(filter: &Node, term: String, page: &Page, mut params: Vec<Param>) -> Compiled {
    // $3 — bound before any clause parameter, like the tenant pair above, so
    // the numbering stays positional and predictable. The term is rebuilt into
    // tsquery text first (see `tsquery_of`), so what is bound is a query, not
    // the raw typing.
    let text = tsquery_of(&term);
    let query = bind(&mut params, Param::Text(text));

    // Every other clause still applies. `Field::Q` emits TRUE inside this
    // shape because it has been hoisted into the FROM and WHERE.
    let predicate = emit(filter, &mut params);

    // The ranking EXPRESSION, not the `rank` alias. A select-list alias is not
    // visible in `WHERE`, so a keyset resume written against `rank` fails with
    // "column rank does not exist" — on the second page only, which is exactly
    // the kind of bug a first-page test never sees.
    let col = if page.sort.field == SortField::Rank {
        RANK
    } else {
        sort_column(page.sort.field)
    };
    let keyset = keyset_predicate(page, &mut params, col);
    let dir = match page.sort.direction {
        Direction::Asc => "ASC",
        Direction::Desc => "DESC",
    };
    let limit = page.limit.saturating_add(1);
    let columns = crate::task::COLUMNS;
    let configuration = crate::search::CONFIGURATION;

    // `ts_rank_cd` per `docs/26` §Weighting. The rank is in the select list so
    // it can be ordered by and carried in a cursor; `SortField::Rank` maps to
    // this alias and exists only in this shape.
    let sql = format!(
        "SELECT {columns}, {RANK} AS rank \
           FROM task_search s \
           JOIN task t ON t.id = s.task_id \
           CROSS JOIN to_tsquery('{configuration}', {query}) q \
          WHERE s.workspace_id = $1 \
            AND s.project_id = ANY($2) \
            AND s.document @@ q \
            AND t.workspace_id = $1 \
            AND t.deleted_at IS NULL \
            AND ({predicate}){keyset} \
          ORDER BY {col} {dir}, t.id {dir} \
          LIMIT {limit}"
    );
    Compiled { sql, params }
}

/// The keyset resume predicate, or nothing on the first page.
///
/// Row-value comparison — `(key, id) < ($k, $id)` — rather than the expanded
/// `key < $k OR (key = $k AND id < $id)`. PostgreSQL can drive a composite
/// index directly from the row-value form; the expanded form is a filter it
/// often cannot, which is the difference between a keyset page and a scan.
/// `col` is the sort EXPRESSION, not a name: in the full-text shape the key is
/// `ts_rank_cd(...)`, and a select-list alias is not visible in `WHERE`.
fn keyset_predicate(page: &Page, params: &mut Vec<Param>, col: &str) -> String {
    let Some(cursor) = &page.after else {
        return String::new();
    };
    // Descending sorts resume *below* the last row, ascending *above* it.
    // Getting this backwards returns the page just served, forever.
    let cmp = match page.sort.direction {
        Direction::Asc => ">",
        Direction::Desc => "<",
    };
    // A cursor is transported as text — it has to be, it is base64url JSON —
    // and every column it resumes against is typed. Without the cast the row
    // comparison is `timestamptz < text`, which is not an operator PostgreSQL
    // has: the whole keyset path fails at execution time, and no test that
    // only inspects the compiler's output can see it. The cast goes on the
    // PARAMETER, for the reason `cast_for` gives — casting the column instead
    // would defeat the index and turn every second page into a scan.
    let key = format!(
        "{}::{}",
        bind(
            params,
            Param::Text(cursor.keys.first().cloned().unwrap_or_default()),
        ),
        cursor_type(page.sort.field)
    );
    let id = format!("{}::uuid", bind(params, Param::Text(cursor.id.to_string())));
    format!(" AND ({col}, t.id) {cmp} ({key}, {id})")
}

/// The PostgreSQL type a cursor's sort key must be cast to.
///
/// Exhaustive, so a new sortable field cannot be added without deciding how its
/// cursor resumes — which is the failure this function exists to prevent.
fn cursor_type(field: SortField) -> &'static str {
    match field {
        SortField::CreatedAt | SortField::UpdatedAt | SortField::DueAt => "timestamptz",
        SortField::Priority => "task_priority",
        SortField::StatusPosition => "integer",
        // `key` sorts by `task.number`, and `position` is already text.
        SortField::Key => "bigint",
        SortField::Position => "text",
        // `rank` is `ts_rank`'s output. Only meaningful with a `q` clause.
        SortField::Rank => "real",
    }
}

/// Push a parameter and return its `$N` placeholder.
fn bind(params: &mut Vec<Param>, p: Param) -> String {
    params.push(p);
    format!("${}", params.len())
}

fn emit(node: &Node, params: &mut Vec<Param>) -> String {
    match node {
        Node::And(children) | Node::Or(children) => {
            let joiner = if matches!(node, Node::And(_)) {
                " AND "
            } else {
                " OR "
            };
            if children.is_empty() {
                // An empty group must not vanish into an empty string, which
                // would produce `AND ()` and a syntax error. `AND` of nothing
                // is true; `OR` of nothing is false.
                return if matches!(node, Node::And(_)) {
                    "TRUE".to_owned()
                } else {
                    "FALSE".to_owned()
                };
            }
            let parts: Vec<String> = children.iter().map(|c| emit(c, params)).collect();
            format!("({})", parts.join(joiner))
        }
        Node::Not(inner) => format!("NOT ({})", emit(inner, params)),
        Node::Clause(c) => emit_clause(c.field, c.op, &c.value, params),
    }
}

fn emit_clause(field: Field, op: Operator, value: &Value, params: &mut Vec<Param>) -> String {
    // Many-to-many and derived fields, as EXISTS rather than JOIN.
    match field {
        Field::Assignee => return emit_assignee(op, value, params),
        Field::Tag => return emit_tag(op, value, params),
        Field::IsBlocked => {
            let p = cast(&bind(params, param_of(value)), Field::IsBlocked, false);
            return format!(
                // `blocked_task_id` is a column `task_dependency` has never
                // had — migration 0005 names the two ends `from_task_id` and
                // `to_task_id`. Every `is_blocked` filter therefore failed at
                // execution time with "column does not exist"; no test caught
                // it because the compiler's own suite asserts the SQL text, and
                // the EXPLAIN catalogue has no probe for this field.
                //
                // "Blocked" is the same question `task::unresolved_blockers`
                // asks: an unresolved BLOCKS edge pointing AT this task.
                // Nested EXISTS rather than a JOIN, so the compiler's simple
                // invariant — no JOIN anywhere in a compiled query — keeps
                // holding. A JOIN inside EXISTS cannot duplicate outer rows,
                // but "no JOIN" is a rule a reader can check at a glance and a
                // narrower one is a rule that erodes.
                "(EXISTS (SELECT 1 FROM task_dependency d \
                  WHERE d.to_task_id = t.id \
                    AND d.kind = 'BLOCKS' \
                    AND EXISTS (SELECT 1 FROM task b \
                                 WHERE b.id = d.from_task_id \
                                   AND b.deleted_at IS NULL \
                                   AND b.state NOT IN ('COMPLETED','CANCELED')))) = ({p})"
            );
        }
        Field::Key => {
            // `key` is the human identifier `WR-125` — `project.key` and
            // `task.number` concatenated, living in two tables. docs/27 lists
            // it as filterable and the committed schema has no column to
            // compare against, so there is nothing correct to emit here.
            //
            // Matching nothing is the safe direction: a `key` filter that
            // returns no rows is visibly wrong to whoever ran it, where one
            // that compared against `t.number` alone would match `WR-125` and
            // `OPS-125` identically and look right. Tracked as **D-051**.
            return "FALSE".to_owned();
        }
        Field::Q => {
            // Already hoisted into the FROM and WHERE by `compile_search`,
            // which is the only path that can reach a `q` clause. Emitting the
            // predicate a second time here would build a second `tsquery` and
            // ask the planner to satisfy the same condition twice.
            return "TRUE".to_owned();
        }
        Field::Archived => {
            // `archived` is a BOOLEAN in the grammar and a TIMESTAMP in the
            // schema: `archived=true` means "has an archived_at", not
            // "archived_at equals the string true". Compiled as a column
            // comparison it bound `'true'::timestamptz` and errored at
            // execution — a filter that 500s rather than answers.
            let p = cast(&bind(params, param_of(value)), Field::Archived, false);
            return format!("(t.archived_at IS NOT NULL) = ({p})");
        }
        _ => {}
    }

    let col = column_of(field);
    match op {
        Operator::IsEmpty => format!("{col} IS NULL"),
        Operator::IsNotEmpty => format!("{col} IS NOT NULL"),
        Operator::Between => match value {
            Value::Range(lo, hi) => {
                let a = cast(&bind(params, Param::Text(lo.clone())), field, false);
                let b = cast(&bind(params, Param::Text(hi.clone())), field, false);
                format!("{col} BETWEEN {a} AND {b}")
            }
            _ => "FALSE".to_owned(),
        },
        Operator::In => {
            let p = cast(&bind(params, param_of(value)), field, true);
            format!("{col} = ANY({p})")
        }
        Operator::NotIn => {
            let p = cast(&bind(params, param_of(value)), field, true);
            format!("NOT ({col} = ANY({p}))")
        }
        Operator::StartsWith | Operator::Contains => {
            let p = bind(params, param_of(value));
            // The wildcard is applied in SQL around a bound parameter, so the
            // user's text is never concatenated into a pattern here.
            if matches!(op, Operator::StartsWith) {
                format!("{col} LIKE {p} || '%'")
            } else {
                format!("{col} LIKE '%' || {p} || '%'")
            }
        }
        _ => {
            let p = cast(&bind(params, param_of(value)), field, false);
            let sql_op = match op {
                Operator::Eq => "=",
                Operator::Gt | Operator::After => ">",
                Operator::Gte => ">=",
                Operator::Lt | Operator::Before => "<",
                Operator::Lte => "<=",
                // Every remaining operator is handled above; a filter that
                // reached here failed validation, and matching nothing is the
                // safe direction.
                _ => return "FALSE".to_owned(),
            };
            format!("{col} {sql_op} {p}")
        }
    }
}

fn emit_assignee(op: Operator, value: &Value, params: &mut Vec<Param>) -> String {
    let field = Field::Assignee;
    let exists = |inner: &str| {
        format!("EXISTS (SELECT 1 FROM task_assignee a WHERE a.task_id = t.id{inner})")
    };
    match op {
        Operator::IsEmpty => format!("NOT {}", exists("")),
        Operator::IsNotEmpty => exists(""),
        _ => {
            let raw = bind(params, param_of(value));
            let cmp = if matches!(op, Operator::In) {
                format!(" AND a.user_id = ANY({})", cast(&raw, field, true))
            } else {
                format!(" AND a.user_id = {}", cast(&raw, field, false))
            };
            exists(&cmp)
        }
    }
}

fn emit_tag(op: Operator, value: &Value, params: &mut Vec<Param>) -> String {
    let exists =
        |inner: &str| format!("EXISTS (SELECT 1 FROM task_tag tt WHERE tt.task_id = t.id{inner})");
    match op {
        Operator::IsEmpty => format!("NOT {}", exists("")),
        _ => {
            let p = cast(&bind(params, param_of(value)), Field::Tag, true);
            let clause = exists(&format!(" AND tt.tag_id = ANY({p})"));
            if matches!(op, Operator::NotIn) {
                format!("NOT {clause}")
            } else {
                clause
            }
        }
    }
}

/// Every value becomes a parameter. This is the only conversion, and it never
/// returns SQL text.
fn param_of(value: &Value) -> Param {
    match value {
        Value::Literal(s) | Value::Symbol(s) => Param::Text(s.clone()),
        Value::List(items) => Param::TextList(items.clone()),
        Value::Range(a, b) => Param::TextList(vec![a.clone(), b.clone()]),
        Value::None => Param::TextList(Vec::new()),
    }
}
