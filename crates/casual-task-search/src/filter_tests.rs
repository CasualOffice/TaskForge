use super::*;

fn clause(field: Field, op: Operator, value: Value) -> Node {
    Node::Clause(Clause { field, op, value })
}

#[test]
fn an_unknown_field_cannot_be_named_at_all() {
    // ADR-011 is enforced by the type: there is no variant to construct.
    assert!(Field::parse("assignee").is_some());
    assert!(Field::parse("salary").is_none());
    assert!(Field::parse("").is_none());
    assert!(Field::parse("ASSIGNEE").is_none(), "matching is exact");
}

#[test]
fn every_field_round_trips_through_its_name() {
    // A field whose name does not parse back is unreachable from the URL
    // form, which would make it silently unfilterable.
    for f in [
        Field::Project,
        Field::Status,
        Field::State,
        Field::Type,
        Field::Priority,
        Field::Assignee,
        Field::Reporter,
        Field::Team,
        Field::Tag,
        Field::Milestone,
        Field::Environment,
        Field::Parent,
        Field::CreatedAt,
        Field::UpdatedAt,
        Field::DueAt,
        Field::Key,
        Field::Title,
        Field::Q,
        Field::IsBlocked,
        Field::Archived,
    ] {
        assert_eq!(Field::parse(f.as_str()), Some(f), "{}", f.as_str());
        assert!(!f.operators().is_empty(), "{} permits nothing", f.as_str());
    }
}

#[test]
fn an_operator_the_field_does_not_permit_is_refused() {
    // docs/27 constraint 2: rejected at parse time, not at the database.
    let n = clause(
        Field::DueAt,
        Operator::Contains,
        Value::Literal("urgent".into()),
    );
    assert_eq!(
        validate(&n),
        Err(FilterError::OperatorNotPermitted {
            field: Field::DueAt,
            op: Operator::Contains
        })
    );
}

#[test]
fn the_operator_table_is_not_uniform_by_type() {
    // These are the asymmetries that deriving operators from the type would
    // silently erase.
    assert!(
        !Field::Reporter.permits(Operator::IsEmpty),
        "a task has one"
    );
    assert!(
        !Field::Tag.permits(Operator::Eq),
        "a tag set is not one value"
    );
    assert!(Field::Key.permits(Operator::StartsWith));
    assert!(!Field::Key.permits(Operator::Contains));
    assert!(Field::Title.permits(Operator::Contains));
    assert!(!Field::Title.permits(Operator::StartsWith));
    assert!(Field::Priority.permits(Operator::Gte), "ordered enum");
    assert!(!Field::State.permits(Operator::Gte), "unordered enum");
}

#[test]
fn a_value_shape_that_does_not_match_the_operator_is_refused() {
    assert!(matches!(
        validate(&clause(
            Field::Assignee,
            Operator::IsEmpty,
            Value::Literal("someone".into())
        )),
        Err(FilterError::MalformedValue { .. })
    ));
    assert!(matches!(
        validate(&clause(
            Field::CreatedAt,
            Operator::Between,
            Value::Literal("-7d".into())
        )),
        Err(FilterError::MalformedValue { .. })
    ));
    assert!(
        validate(&clause(
            Field::CreatedAt,
            Operator::Between,
            Value::Range("-7d".into(), "@today".into())
        ))
        .is_ok()
    );
}

#[test]
fn symbolic_values_survive_validation_unresolved() {
    // docs/27: resolved at evaluation, so a saved view stays correct as
    // context changes. Validation must not require them to be concrete.
    assert!(
        validate(&clause(
            Field::Assignee,
            Operator::Eq,
            Value::Symbol("@me".into())
        ))
        .is_ok()
    );
    assert!(
        validate(&clause(
            Field::DueAt,
            Operator::Before,
            Value::Symbol("+7d".into())
        ))
        .is_ok()
    );
}

#[test]
fn the_clause_and_depth_bounds_are_enforced() {
    let one = || clause(Field::State, Operator::Eq, Value::Literal("ACTIVE".into()));

    let wide = Node::And((0..MAX_CLAUSES + 1).map(|_| one()).collect());
    assert_eq!(
        validate(&wide),
        Err(FilterError::TooManyClauses(MAX_CLAUSES + 1))
    );
    assert!(validate(&Node::And((0..MAX_CLAUSES).map(|_| one()).collect())).is_ok());

    let mut deep = one();
    for _ in 0..MAX_DEPTH {
        deep = Node::Not(Box::new(deep));
    }
    assert!(matches!(validate(&deep), Err(FilterError::TooDeep(_))));
}

#[test]
fn bounds_are_reported_before_shape() {
    // A filter that is both enormous and malformed should report the cheap
    // structural reason — a user fixing 33 clauses down to 32 will then see
    // the real problem, and the reverse order wastes a round trip.
    let bad = clause(Field::Title, Operator::Matches, Value::Literal("x".into()));
    let mut nodes: Vec<Node> = (0..MAX_CLAUSES).map(|_| bad.clone()).collect();
    nodes.push(bad);
    assert_eq!(
        validate(&Node::And(nodes)),
        Err(FilterError::TooManyClauses(MAX_CLAUSES + 1))
    );
}

#[test]
fn the_bound_errors_carry_their_registered_codes() {
    assert_eq!(
        FilterError::TooManyClauses(33).code(),
        codes::QRY_TOO_MANY_CLAUSES
    );
    assert_eq!(FilterError::TooDeep(5).code(), codes::QRY_TOO_DEEP);
    assert_eq!(
        FilterError::UnknownField("salary".into()).code(),
        codes::QRY_UNKNOWN_FIELD
    );
    assert_eq!(
        FilterError::OperatorNotPermitted {
            field: Field::Title,
            op: Operator::Matches
        }
        .code(),
        codes::QRY_BAD_OPERATOR
    );
}
