use super::*;
use crate::url;

fn parse(text: &str) -> Result<Node, JsonError> {
    from_json(&serde_json::from_str::<Json>(text).expect("valid JSON"))
}

#[test]
fn the_documented_example_parses() {
    // docs/27 §The AST, character for character.
    let node = parse(
        r#"{
              "op": "and",
              "clauses": [
                { "field": "state",    "op": "in",     "value": ["ACTIVE", "PLANNED"] },
                { "field": "assignee", "op": "eq",     "value": "@me" },
                { "field": "due_at",   "op": "before", "value": "+7d" },
                {
                  "op": "or",
                  "clauses": [
                    { "field": "priority", "op": "gte", "value": "HIGH" },
                    { "field": "tag",      "op": "in",  "value": ["security"] }
                  ]
                }
              ]
            }"#,
    )
    .expect("the documented example must parse");

    let Node::And(children) = &node else {
        panic!("expected a top-level and");
    };
    assert_eq!(children.len(), 4);
    assert!(
        matches!(children[3], Node::Or(_)),
        "the nested group survives"
    );
}

#[test]
fn a_symbol_stays_a_symbol() {
    // The failure this closes: `@me` reaching the database as three
    // characters, which `resolve`'s own docs call out.
    let node = parse(r#"{ "field": "assignee", "op": "eq", "value": "@me" }"#).expect("parses");
    assert_eq!(
        node,
        Node::Clause(Clause {
            field: Field::Assignee,
            op: Operator::Eq,
            value: Value::Symbol("@me".into()),
        })
    );
}

#[test]
fn both_surfaces_classify_a_symbol_identically() {
    // docs/27 §Compilation: one AST, two entry points. If these disagreed,
    // the same saved view would mean different things depending on whether
    // it arrived as a link or as stored JSON.
    for raw in ["@me", "@today", "+7d", "-3mo", "WR-125", "7d", "HIGH"] {
        let from_url = url::parse([("title", raw)]).expect("url parses");
        let Node::And(clauses) = &from_url.filter else {
            panic!("flat and")
        };
        let Node::Clause(url_clause) = &clauses[0] else {
            panic!("clause")
        };
        let json = parse(&format!(
            r#"{{ "field": "title", "op": "contains", "value": {} }}"#,
            serde_json::to_string(raw).expect("string")
        ))
        .expect("json parses");
        let Node::Clause(json_clause) = &json else {
            panic!("clause")
        };
        assert_eq!(
            std::mem::discriminant(&url_clause.value),
            std::mem::discriminant(&json_clause.value),
            "{raw:?} was classified differently by the two surfaces"
        );
    }
}

#[test]
fn the_operator_decides_between_a_list_and_a_range() {
    // Both are arrays of strings. Only the operator tells them apart.
    let list = parse(r#"{ "field": "state", "op": "in", "value": ["ACTIVE","PLANNED"] }"#)
        .expect("in is a list");
    let Node::Clause(list) = list else { panic!() };
    assert!(matches!(list.value, Value::List(_)));

    let range = parse(r#"{ "field": "due_at", "op": "between", "value": ["@today","+7d"] }"#)
        .expect("between is a range");
    let Node::Clause(range) = range else { panic!() };
    assert_eq!(range.value, Value::Range("@today".into(), "+7d".into()));
}

#[test]
fn a_between_without_two_bounds_is_refused() {
    for text in [
        r#"{ "field": "due_at", "op": "between", "value": ["@today"] }"#,
        r#"{ "field": "due_at", "op": "between", "value": "@today" }"#,
        r#"{ "field": "due_at", "op": "between", "value": ["a","b","c"] }"#,
    ] {
        assert!(parse(text).is_err(), "accepted {text}");
    }
}

#[test]
fn an_operator_the_field_forbids_is_refused() {
    // docs/27 constraint 2: rejected at parse time, not at the database.
    // `title` permits only `contains`.
    let error =
        parse(r#"{ "field": "title", "op": "gt", "value": "x" }"#).expect_err("not permitted");
    assert_eq!(error.code(), codes::QRY_BAD_OPERATOR);
}

#[test]
fn an_unknown_field_is_refused_rather_than_ignored() {
    // The dangerous direction: a dropped clause returns MORE rows than asked.
    let error = parse(r#"{ "field": "colour", "op": "eq", "value": "red" }"#).expect_err("unknown");
    assert_eq!(error, JsonError::UnknownField("colour".into()));
    assert_eq!(error.code(), codes::QRY_UNKNOWN_FIELD);
}

#[test]
fn is_empty_takes_no_value_and_refuses_one() {
    let node = parse(r#"{ "field": "assignee", "op": "is_empty" }"#).expect("no value");
    let Node::Clause(clause) = node else { panic!() };
    assert_eq!(clause.value, Value::None);

    assert!(
        parse(r#"{ "field": "assignee", "op": "is_empty", "value": "x" }"#).is_err(),
        "a value beside is_empty is a filter somebody believes is narrower than it is"
    );
}

#[test]
fn an_empty_set_is_refused() {
    assert!(parse(r#"{ "field": "state", "op": "in", "value": [] }"#).is_err());
}

#[test]
fn not_takes_exactly_one_child() {
    assert!(
        parse(r#"{ "op": "not", "clauses": [{"field":"state","op":"eq","value":"DONE"}] }"#)
            .is_ok()
    );
    assert!(
        parse(r#"{ "op": "not", "clauses": [] }"#).is_err(),
        "an empty `not` has nothing to negate"
    );
    assert!(
        parse(
            r#"{ "op": "not", "clauses": [
                     {"field":"state","op":"eq","value":"DONE"},
                     {"field":"type","op":"eq","value":"BUG"}] }"#
        )
        .is_err(),
        "implicitly and-ing would make `not` mean two things"
    );
}

#[test]
fn ast_to_json_to_ast_is_identity() {
    // docs/27 §Acceptance gates, in as many shapes as the AST has.
    let trees = [
        Node::And(vec![
            Node::Clause(Clause {
                field: Field::State,
                op: Operator::In,
                value: Value::List(vec!["ACTIVE".into(), "PLANNED".into()]),
            }),
            Node::Clause(Clause {
                field: Field::Assignee,
                op: Operator::Eq,
                value: Value::Symbol("@me".into()),
            }),
            Node::Clause(Clause {
                field: Field::Assignee,
                op: Operator::IsEmpty,
                value: Value::None,
            }),
            Node::Clause(Clause {
                field: Field::DueAt,
                op: Operator::Between,
                value: Value::Range("@today".into(), "+7d".into()),
            }),
            Node::Not(Box::new(Node::Clause(Clause {
                field: Field::Type,
                op: Operator::Eq,
                value: Value::Literal("BUG".into()),
            }))),
            Node::Or(vec![Node::Clause(Clause {
                field: Field::Priority,
                op: Operator::Gte,
                value: Value::Literal("HIGH".into()),
            })]),
        ]),
        Node::Clause(Clause {
            field: Field::Q,
            op: Operator::Matches,
            value: Value::Literal("payment retry".into()),
        }),
    ];

    for tree in trees {
        assert_eq!(from_json(&to_json(&tree)), Ok(tree.clone()), "{tree:?}");
    }
}

#[test]
fn the_url_surface_and_the_json_surface_meet_at_the_same_ast() {
    // The whole promise of §Compilation, asserted rather than assumed.
    let from_url = url::parse([
        ("state", "ACTIVE,PLANNED"),
        ("assignee", "@me"),
        ("due_at", "<+7d"),
    ])
    .expect("url parses");
    let round_tripped = from_json(&to_json(&from_url.filter)).expect("json round trip");
    assert_eq!(round_tripped, from_url.filter);
}
