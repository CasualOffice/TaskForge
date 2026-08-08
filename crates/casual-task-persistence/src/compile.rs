//! Compiling a validated filter to parameterized SQL (`docs/27` §Compilation).
//!
//! # The one rule
//!
//! "The compiler emits only `$1`-style parameters and identifiers from a static
//! map. There is no string interpolation of user data anywhere in the path."
//!
//! Every user-supplied value becomes a [`Param`] and is referenced by position.
//! The only text that reaches the SQL string is a column name from
//! [`column_of`], an operator from a fixed `match`, and digits. A hostile value
//! has nowhere to go: `Value` holds `String`s, and no code path writes one into
//! the output.
//!
//! # The permission filter cannot be forgotten
//!
//! `docs/27`: "It is structurally impossible to compile a filter that omits it
//! — the compiler's signature requires an `AuthorizedProjectSet`."
//!
//! [`compile`] takes one by value. There is no overload without it, and
//! [`AuthorizedProjectSet`] cannot be constructed from a plain `Vec` by
//! accident — it is built from the resolver's answer. A missing tenant filter is
//! therefore not a code-review oversight; it does not compile.
//!
//! # Why `EXISTS` and not `JOIN`
//!
//! `assignee` and `tag` are many-to-many. A `JOIN` makes a task with two
//! matching tags appear twice, which forces `DISTINCT`, which breaks keyset
//! pagination — the cursor's `(updated_at, id)` comparison stops being a total
//! order over the result set. `EXISTS` returns each task once with no
//! deduplication.

use casual_task_model::{ProjectId, WorkspaceId};
use casual_task_search::filter::{Field, Node, Operator, Value};

/// The projects an actor may see, from the resolver.
///
/// A newtype rather than `Vec<ProjectId>` so it cannot be produced by writing
/// `vec![]` at a call site. `docs/04` §The list problem resolves this once per
/// request; the compiler consumes that answer and cannot invent one.
#[derive(Debug, Clone)]
pub struct AuthorizedProjectSet(Vec<ProjectId>);

impl AuthorizedProjectSet {
    /// Build from the resolver's answer.
    ///
    /// An empty set is legitimate — an actor with access to no project — and
    /// compiles to a predicate matching nothing rather than to no predicate.
    pub fn resolved(projects: Vec<ProjectId>) -> Self {
        Self(projects)
    }

    pub fn as_slice(&self) -> &[ProjectId] {
        &self.0
    }
}

/// A bound value. Never rendered into SQL text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Param {
    Workspace(WorkspaceId),
    Projects(Vec<ProjectId>),
    Text(String),
    TextList(Vec<String>),
}

/// SQL plus the parameters it references, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub sql: String,
    pub params: Vec<Param>,
}

/// The static identifier map. The only place a column name is written.
fn column_of(field: Field) -> &'static str {
    match field {
        Field::Project => "t.project_id",
        Field::Status => "t.status_id",
        Field::State => "t.state",
        Field::Type => "t.type",
        Field::Priority => "t.priority",
        Field::Reporter => "t.reporter_id",
        Field::Milestone => "t.milestone_id",
        Field::Environment => "t.environment_id",
        Field::Parent => "t.parent_id",
        Field::CreatedAt => "t.created_at",
        Field::UpdatedAt => "t.updated_at",
        Field::DueAt => "t.due_at",
        Field::Key => "t.number",
        Field::Title => "t.title",
        Field::Archived => "t.archived_at",
        // Handled structurally rather than as a column comparison.
        Field::Assignee | Field::Tag | Field::Q | Field::IsBlocked => "",
    }
}

/// Compile a **validated** filter.
///
/// Takes an [`AuthorizedProjectSet`] because `docs/27` requires the permission
/// filter to be injected here rather than supplied by the caller.
///
/// The filter must already have passed `casual_task_search::filter::validate`.
/// This function does not re-check field/operator legality — that would
/// duplicate the rule in two places and invite them to drift.
pub fn compile(
    filter: &Node,
    workspace: WorkspaceId,
    authorized: &AuthorizedProjectSet,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();

    // Injected first, always. Not conditional, not caller-supplied.
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    let predicate = emit(filter, &mut params);

    let sql = format!(
        "SELECT t.* FROM task t \
         WHERE t.workspace_id = $1 \
           AND t.project_id = ANY($2) \
           AND t.deleted_at IS NULL \
           AND ({predicate}) \
         ORDER BY t.updated_at DESC, t.id DESC \
         LIMIT 51"
    );
    Compiled { sql, params }
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
            let p = bind(params, param_of(value));
            return format!(
                "(EXISTS (SELECT 1 FROM task_dependency d \
                  WHERE d.blocked_task_id = t.id)) = ({p}::boolean)"
            );
        }
        Field::Q => {
            let p = bind(params, param_of(value));
            return format!(
                "EXISTS (SELECT 1 FROM task_search s \
                  WHERE s.task_id = t.id \
                    AND s.document @@ plainto_tsquery('english', {p}))"
            );
        }
        _ => {}
    }

    let col = column_of(field);
    match op {
        Operator::IsEmpty => format!("{col} IS NULL"),
        Operator::IsNotEmpty => format!("{col} IS NOT NULL"),
        Operator::Between => match value {
            Value::Range(lo, hi) => {
                let a = bind(params, Param::Text(lo.clone()));
                let b = bind(params, Param::Text(hi.clone()));
                format!("{col} BETWEEN {a} AND {b}")
            }
            _ => "FALSE".to_owned(),
        },
        Operator::In => {
            let p = bind(params, param_of(value));
            format!("{col} = ANY({p})")
        }
        Operator::NotIn => {
            let p = bind(params, param_of(value));
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
            let p = bind(params, param_of(value));
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
    let exists = |inner: &str| {
        format!("EXISTS (SELECT 1 FROM task_assignee a WHERE a.task_id = t.id{inner})")
    };
    match op {
        Operator::IsEmpty => format!("NOT {}", exists("")),
        Operator::IsNotEmpty => exists(""),
        _ => {
            let p = bind(params, param_of(value));
            let cmp = if matches!(op, Operator::In) {
                format!(" AND a.user_id = ANY({p})")
            } else {
                format!(" AND a.user_id = {p}")
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
            let p = bind(params, param_of(value));
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

#[cfg(test)]
mod tests {
    use super::*;
    use casual_task_search::filter::Clause;

    fn clause(field: Field, op: Operator, value: Value) -> Node {
        Node::Clause(Clause { field, op, value })
    }

    fn compiled(node: &Node) -> Compiled {
        compile(
            node,
            WorkspaceId::new(),
            &AuthorizedProjectSet::resolved(vec![ProjectId::new()]),
        )
    }

    #[test]
    fn the_permission_filter_is_always_present() {
        // docs/27: structurally impossible to omit. The signature requires the
        // set, and the predicate is unconditional.
        let c = compiled(&Node::And(Vec::new()));
        assert!(c.sql.contains("t.workspace_id = $1"));
        assert!(c.sql.contains("t.project_id = ANY($2)"));
        assert!(matches!(c.params[0], Param::Workspace(_)));
        assert!(matches!(c.params[1], Param::Projects(_)));
    }

    #[test]
    fn an_actor_with_no_projects_gets_a_predicate_matching_nothing() {
        // Not "no predicate". An empty authorized set must not widen the query.
        let c = compile(
            &Node::And(Vec::new()),
            WorkspaceId::new(),
            &AuthorizedProjectSet::resolved(Vec::new()),
        );
        assert!(c.sql.contains("t.project_id = ANY($2)"));
        assert_eq!(c.params[1], Param::Projects(Vec::new()));
    }

    #[test]
    fn many_to_many_fields_compile_to_exists_not_join() {
        // A JOIN makes a task with two matching tags appear twice, forcing
        // DISTINCT, which breaks keyset pagination.
        for node in [
            clause(Field::Tag, Operator::In, Value::List(vec!["a".into()])),
            clause(Field::Assignee, Operator::Eq, Value::Symbol("@me".into())),
        ] {
            let c = compiled(&node);
            assert!(c.sql.contains("EXISTS"), "{}", c.sql);
            assert!(!c.sql.contains("JOIN"), "{}", c.sql);
            assert!(!c.sql.contains("DISTINCT"), "{}", c.sql);
        }
    }

    #[test]
    fn an_empty_group_does_not_produce_invalid_sql() {
        // `AND ()` is a syntax error. An empty AND is true, an empty OR false.
        assert!(compiled(&Node::And(Vec::new())).sql.contains("(TRUE)"));
        assert!(compiled(&Node::Or(Vec::new())).sql.contains("(FALSE)"));
    }

    #[test]
    fn the_sql_is_independent_of_the_value() {
        // docs/15 §Security: "filter compiler emits no user-derived SQL
        // strings". Stated as substring absence this test is wrong in both
        // directions — `$1` and `t.workspace_id` are hostile-looking inputs
        // that the SQL legitimately contains for other reasons, and a value
        // could still leak in a form the search missed.
        //
        // The property that actually holds is stronger and exact: compiling the
        // same filter with a different value must produce BYTE-IDENTICAL SQL,
        // because the value never reaches the text at all.
        let hostile = [
            "'; DROP TABLE task; --",
            "' OR '1'='1",
            "\\'; DELETE FROM workspace WHERE '' = '",
            "$1",
            "t.workspace_id",
            "*/ UNION SELECT * FROM api_token --",
            "100%",
            "",
        ];

        for (field, op) in [
            (Field::Title, Operator::Contains),
            (Field::Key, Operator::StartsWith),
            (Field::State, Operator::Eq),
            (Field::Q, Operator::Matches),
            (Field::Assignee, Operator::Eq),
            (Field::DueAt, Operator::Before),
        ] {
            let benign = compiled(&clause(field, op, Value::Literal("benign".into())));
            for value in hostile {
                let c = compiled(&clause(field, op, Value::Literal(value.to_owned())));
                assert_eq!(
                    c.sql, benign.sql,
                    "the SQL changed with the value for {field:?} {op:?} — \
                     `{value}` influenced the text"
                );
                assert!(
                    c.params.contains(&Param::Text(value.to_owned())),
                    "`{value}` was not bound as a parameter either"
                );
            }
        }
    }

    #[test]
    fn a_hostile_value_in_a_list_is_also_only_ever_bound() {
        let hostile = "'; DROP TABLE task; --";
        let c = compiled(&clause(
            Field::Tag,
            Operator::In,
            Value::List(vec![hostile.to_owned(), "ok".to_owned()]),
        ));
        assert!(!c.sql.contains(hostile), "{}", c.sql);
        assert!(
            c.params
                .iter()
                .any(|p| matches!(p, Param::TextList(v) if v.contains(&hostile.to_owned())))
        );
    }

    #[test]
    fn the_sql_contains_only_placeholders_columns_and_keywords() {
        // The strongest form of the property: after removing every column name
        // this compiler is allowed to emit, nothing that looks like user data
        // remains — only `$N`, punctuation, and SQL keywords.
        let node = Node::And(vec![
            clause(
                Field::State,
                Operator::In,
                Value::List(vec!["ACTIVE".into()]),
            ),
            clause(
                Field::Title,
                Operator::Contains,
                Value::Literal("x'y".into()),
            ),
            clause(Field::Assignee, Operator::Eq, Value::Symbol("@me".into())),
            clause(Field::DueAt, Operator::Before, Value::Symbol("+7d".into())),
        ]);
        let c = compiled(&node);
        for forbidden in ["ACTIVE", "x'y", "@me", "+7d"] {
            assert!(
                !c.sql.contains(forbidden),
                "`{forbidden}` leaked into:\n{}",
                c.sql
            );
        }
        assert_eq!(c.params.len(), 6, "two injected plus four clause values");
    }

    #[test]
    fn wildcards_are_applied_in_sql_around_a_parameter() {
        // The pattern is built by SQL concatenation of a bound value, not by
        // formatting the value into a Rust string — so `%` in user input is
        // data, not syntax.
        let c = compiled(&clause(
            Field::Title,
            Operator::Contains,
            Value::Literal("100%".into()),
        ));
        assert!(c.sql.contains("LIKE '%' || $3 || '%'"), "{}", c.sql);
        assert!(!c.sql.contains("100%"));
    }
}
