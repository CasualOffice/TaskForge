use super::tsquery_of;

/// The failure these prevent: an empty result for every keystroke before the
/// last, and a tsquery syntax error from ordinary punctuation.
mod prefix_search {
    use super::tsquery_of;

    #[test]
    fn the_last_token_is_a_prefix_and_the_others_are_not() {
        // The last token is the one being typed. Making them all prefixes
        // would match far more than was asked for.
        assert_eq!(tsquery_of("restore backu"), "restore & backu:*",);
    }

    #[test]
    fn a_single_word_is_a_prefix() {
        // This is the whole point: `backu` has to find "Backup restore
        // drill" before the word is finished.
        assert_eq!(tsquery_of("backu"), "backu:*");
    }

    #[test]
    fn a_task_key_keeps_its_hyphen() {
        // `OPS-1` is the search people use most. Dropping the hyphen with
        // the rest of the punctuation would break it.
        assert_eq!(tsquery_of("OPS-1"), "OPS-1:*");
    }

    #[test]
    fn tsquery_operators_in_the_typing_are_dropped_not_escaped() {
        // `to_tsquery` parses its argument as syntax, so `&`, `|`, `!`, the
        // parens and the colon are operators unless they never arrive. The
        // only operators in the output are the ones this function adds.
        let text = tsquery_of("a & b | !c (d) e:f");
        // `e:f` is one whitespace-delimited token and reduces to `ef`. The
        // colon does not survive to separate them, which is the point: a
        // `:` reaching `to_tsquery` is a weight or prefix marker, not text.
        assert_eq!(text, "a & b & c & d & ef:*");
    }

    #[test]
    fn punctuation_alone_never_becomes_a_token() {
        // A token reduced to `-` would emit `-:*`. Requiring one
        // alphanumeric is what stops that.
        assert_eq!(tsquery_of("backup - restore"), "backup & restore:*",);
    }

    #[test]
    fn text_that_tokenizes_to_nothing_binds_an_empty_query() {
        // No user-supplied operator reaches `to_tsquery`. PostgreSQL turns
        // the empty argument into an empty query that matches nothing.
        assert_eq!(tsquery_of("!!!"), "");
        assert_eq!(tsquery_of(""), "");
    }

    #[test]
    fn a_name_outside_ascii_is_still_a_term() {
        // `is_alphanumeric` is Unicode-aware, and a colleague's name is the
        // most likely non-ASCII thing anybody types here.
        assert_eq!(tsquery_of("Bekele"), "Bekele:*");
        assert_eq!(tsquery_of("Ökafor"), "Ökafor:*");
    }
}

use super::*;
use casual_task_search::filter::Clause;

fn clause(field: Field, op: Operator, value: Value) -> Node {
    Node::Clause(Clause { field, op, value })
}

fn compiled(node: &Node) -> Compiled {
    paged(node, &Page::default())
}

fn paged(node: &Node, page: &Page) -> Compiled {
    compile(
        node,
        WorkspaceId::new(),
        &AuthorizedProjectSet::resolved(vec![ProjectId::new()]),
        page,
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
        &Page::default(),
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

    // `key` is excluded deliberately: it compiles to FALSE pending D-051
    // and binds nothing, so "was it bound as a parameter" cannot hold. It
    // is covered by its own test below.
    for (field, op) in [
        (Field::Title, Operator::Contains),
        (Field::State, Operator::Eq),
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

    // Search deliberately rebuilds the raw typing as tsquery text before
    // binding it. The SQL remains value-independent, while the bound value
    // is the sanitized derivative rather than the hostile input itself.
    let benign = compiled(&clause(
        Field::Q,
        Operator::Matches,
        Value::Literal("benign".into()),
    ));
    for value in hostile {
        let c = compiled(&clause(
            Field::Q,
            Operator::Matches,
            Value::Literal(value.to_owned()),
        ));
        assert_eq!(c.sql, benign.sql, "the search SQL changed with `{value}`");
        assert!(
            c.params.contains(&Param::Text(tsquery_of(value))),
            "the sanitized search value derived from `{value}` was not bound"
        );
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
fn the_first_page_has_no_resume_predicate_and_fetches_one_extra() {
    let c = paged(&Node::And(Vec::new()), &Page::default());
    assert!(
        !c.sql.contains(", t.id) <"),
        "no cursor, no keyset: {}",
        c.sql
    );
    // 50 requested, 51 fetched — the extra row answers "is there a next
    // page" without a COUNT over the result set.
    assert!(c.sql.ends_with("LIMIT 51"), "{}", c.sql);
}

#[test]
fn a_cursor_resumes_with_a_row_value_comparison_including_the_tiebreaker() {
    let page = Page {
        after: Some(Cursor::new(
            vec!["2026-08-08T00:00:00Z".into()],
            uuid::Uuid::now_v7(),
        )),
        ..Default::default()
    };
    let c = paged(&Node::And(Vec::new()), &page);
    // Row-value form, and the id tiebreaker is present. docs/26 makes the
    // tiebreaker mandatory: without it, ties in updated_at make the cursor
    // non-deterministic and a page can repeat or skip a row.
    assert!(
        c.sql
            .contains("(t.updated_at, t.id) < ($3::timestamptz, $4::uuid)"),
        "{}",
        c.sql
    );
}

#[test]
fn every_cursor_key_is_cast_to_the_type_of_the_column_it_resumes_against() {
    // A cursor travels as text. Comparing text to a timestamptz column is
    // not an operator PostgreSQL has, so without these casts the second
    // page of every list fails at execution time — a failure no assertion
    // over the compiler's output can see.
    let cursor = Cursor::new(vec!["x".into()], uuid::Uuid::now_v7());
    for (field, ty) in [
        (SortField::CreatedAt, "timestamptz"),
        (SortField::UpdatedAt, "timestamptz"),
        (SortField::DueAt, "timestamptz"),
        (SortField::Priority, "task_priority"),
        (SortField::StatusPosition, "integer"),
        (SortField::Key, "bigint"),
        (SortField::Position, "text"),
        (SortField::Rank, "real"),
    ] {
        let page = Page {
            sort: Sort {
                field,
                direction: Direction::Desc,
            },
            after: Some(cursor.clone()),
            limit: 10,
        };
        let c = paged(&Node::And(Vec::new()), &page);
        assert!(
            c.sql.contains(&format!("::{ty}, $4::uuid)")),
            "{field:?} resumes without a {ty} cast: {}",
            c.sql
        );
    }
}

#[test]
fn the_resume_comparison_follows_the_sort_direction() {
    // Backwards here returns the page just served, forever.
    let cursor = Cursor::new(vec!["x".into()], uuid::Uuid::now_v7());
    for (direction, expected) in [(Direction::Desc, "<"), (Direction::Asc, ">")] {
        let page = Page {
            sort: Sort {
                field: SortField::DueAt,
                direction,
            },
            after: Some(cursor.clone()),
            limit: 10,
        };
        let c = paged(&Node::And(Vec::new()), &page);
        assert!(
            c.sql.contains(&format!("(t.due_at, t.id) {expected} (")),
            "{direction:?} should resume with `{expected}`: {}",
            c.sql
        );
        let dir_sql = if matches!(direction, Direction::Asc) {
            "ASC"
        } else {
            "DESC"
        };
        assert!(
            c.sql
                .contains(&format!("ORDER BY t.due_at {dir_sql}, t.id {dir_sql}")),
            "the tiebreaker must order the same way as the key: {}",
            c.sql
        );
    }
}

#[test]
fn no_page_ever_compiles_to_offset() {
    // docs/26 bans it outright, and the architecture lint bans the token —
    // this asserts the compiler cannot emit one by any route.
    let cursor = Cursor::new(vec!["x".into()], uuid::Uuid::now_v7());
    for after in [None, Some(cursor)] {
        let c = paged(
            &Node::And(Vec::new()),
            &Page {
                after,
                ..Default::default()
            },
        );
        assert!(!c.sql.to_uppercase().contains("OFFSET"), "{}", c.sql);
    }
}

#[test]
fn the_cursor_values_are_bound_not_interpolated() {
    let hostile = "'; DROP TABLE task; --";
    let page = Page {
        after: Some(Cursor::new(vec![hostile.into()], uuid::Uuid::now_v7())),
        ..Default::default()
    };
    let c = paged(&Node::And(Vec::new()), &page);
    assert!(!c.sql.contains(hostile), "{}", c.sql);
    assert!(c.params.contains(&Param::Text(hostile.to_owned())));
}

#[test]
fn every_column_the_compiler_names_exists_in_the_schema() {
    // The bug this would have caught: `is_blocked` emitted
    // `d.blocked_task_id`, a column `task_dependency` has never had —
    // migration 0005 names the ends `from_task_id` and `to_task_id`. Both
    // `?is_blocked=true` and `?is_blocked=false` returned TF-SYS-0001, and
    // the built-in "My Work · Blocked" view could not ship.
    //
    // Nothing caught it because every other test here asserts the SQL
    // *text*, which a wrong column name satisfies perfectly. This reads the
    // migration instead.
    let schema = include_str!("../../../migrations/0005_tasks.sql");
    let mut compiled = String::new();
    for (field, op, value) in [
        (
            Field::IsBlocked,
            Operator::Eq,
            Value::Literal("true".into()),
        ),
        (Field::Assignee, Operator::Eq, Value::Literal("x".into())),
        (Field::Tag, Operator::In, Value::List(vec!["x".into()])),
        (Field::Q, Operator::Matches, Value::Literal("x".into())),
    ] {
        compiled.push_str(&compiled_sql_of(field, op, value));
        compiled.push(' ');
    }

    // Every `<alias>.<column>` the compiler emitted, for the tables whose
    // DDL lives in this migration.
    for token in compiled.split_whitespace() {
        let token =
            token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '_');
        let Some((alias, column)) = token.split_once('.') else {
            continue;
        };
        // Aliases used for tables defined in migration 0005.
        if !matches!(alias, "d" | "a" | "tt") || column.is_empty() {
            continue;
        }
        assert!(
            schema.contains(column),
            "the compiler emits `{alias}.{column}`, which migration 0005 \
                 does not define — every query using it fails at execution time"
        );
    }
}

include!("compile_predicate_tests.rs");
