//! The AST's JSON surface (`docs/27` §The AST, §Compilation).
//!
//! # Why this exists
//!
//! `docs/27` §Compilation draws one pipeline with **two** entry points — a URL
//! query string and a JSON tree — meeting at the same AST before validation and
//! compilation. Only the URL half existed. That was survivable while every
//! filter came from a link, and it stops being survivable the moment a filter
//! has to be *stored*: `docs/27` §Saved views defines a saved view as a JSON
//! tree, precisely because "the URL form expresses only flat `AND`" and a stored
//! view has to be able to hold a nested group.
//!
//! So this is the second door, and it is deliberately a door to the *same* room.
//!
//! # The two ambiguities JSON has and the URL form does not, and how each is closed
//!
//! **A symbol and a literal are both strings.** `"@me"` and `"WR-125"` are
//! indistinguishable in JSON, and getting it wrong is not cosmetic — a symbol
//! left unresolved reaches the database as the four characters `"@me"`, which is
//! the failure [`crate::resolve`]'s own documentation calls out. Both surfaces
//! therefore classify through [`filter::is_symbolic`], so `@me` means the same
//! thing whichever door it came through.
//!
//! **A list and a range are both arrays.** `["ACTIVE","PLANNED"]` and
//! `["@today","+7d"]` have the same JSON shape. The *operator* disambiguates
//! them: `between` takes a range and everything else takes a list. That is not a
//! convention invented here — it is what [`filter::Field::operators`] already
//! permits, so a value shaped one way under an operator that wants the other is
//! refused rather than guessed at.
//!
//! # What this module does not do
//!
//! Validate. [`crate::validate`] enforces the clause and depth limits, and it
//! runs after either surface. Parsing here refuses only what it cannot
//! *represent* — an unknown field, an unknown operator, a value of the wrong
//! shape — because those are the things that would otherwise become a silently
//! different filter.

use serde_json::{Map, Value as Json};

use crate::filter::{Clause, Field, Node, Operator, Value, is_symbolic};
use casual_task_model::ErrorCode;
use casual_task_model::error::codes;

/// Why a JSON tree could not be read as an AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    /// A node that is neither a group nor a clause.
    NotANode,
    UnknownField(String),
    UnknownOperator(String),
    /// The operator and the value shape disagree — `between` without two bounds,
    /// `in` with a bare string, `is_empty` with a value.
    ValueShape {
        field: Field,
        op: Operator,
    },
    /// The operator parsed and the field does not permit it (`docs/27` §Fields).
    OperatorNotPermitted {
        field: Field,
        op: Operator,
    },
    /// `not` takes exactly one child; `and`/`or` take at least one.
    GroupArity(&'static str),
}

impl JsonError {
    /// The registry code (`docs/20`) this refusal reports as.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::UnknownField(_) => codes::QRY_UNKNOWN_FIELD,
            Self::NotANode
            | Self::UnknownOperator(_)
            | Self::ValueShape { .. }
            | Self::OperatorNotPermitted { .. }
            | Self::GroupArity(_) => codes::QRY_BAD_OPERATOR,
        }
    }

    /// The field the caller got wrong, for the error's `details`.
    #[must_use]
    pub fn field(&self) -> String {
        match self {
            Self::UnknownField(name) => name.clone(),
            Self::UnknownOperator(name) => name.clone(),
            Self::ValueShape { field, .. } | Self::OperatorNotPermitted { field, .. } => {
                field.as_str().to_owned()
            }
            Self::NotANode => "node".to_owned(),
            Self::GroupArity(which) => (*which).to_owned(),
        }
    }
}

/// Read a stored JSON tree into an AST.
///
/// # Errors
///
/// [`JsonError`] for a node that is not a group or a clause, an unknown field or
/// operator, an operator the field does not permit, or a value whose shape the
/// operator cannot take.
pub fn from_json(json: &Json) -> Result<Node, JsonError> {
    let object = json.as_object().ok_or(JsonError::NotANode)?;

    // A group is anything carrying `clauses`. Checked before `field`, because a
    // group also carries `op` and reading it as a clause would look for a field
    // that is not there and report the wrong thing.
    if let Some(children) = object.get("clauses") {
        return group(object, children);
    }
    clause(object).map(Node::Clause)
}

fn group(object: &Map<String, Json>, children: &Json) -> Result<Node, JsonError> {
    let op = object
        .get("op")
        .and_then(Json::as_str)
        .ok_or(JsonError::NotANode)?;
    let items = children.as_array().ok_or(JsonError::NotANode)?;
    let nodes = items.iter().map(from_json).collect::<Result<Vec<_>, _>>()?;

    match op {
        "and" | "or" => {
            if nodes.is_empty() {
                return Err(JsonError::GroupArity("clauses"));
            }
            Ok(if op == "and" {
                Node::And(nodes)
            } else {
                Node::Or(nodes)
            })
        }
        // `Not` holds exactly one child. Accepting a list and implicitly
        // `and`-ing it would make `not` mean two different things depending on
        // how many clauses somebody put in it.
        "not" => match <[Node; 1]>::try_from(nodes) {
            Ok([inner]) => Ok(Node::Not(Box::new(inner))),
            Err(_) => Err(JsonError::GroupArity("not")),
        },
        other => Err(JsonError::UnknownOperator(other.to_owned())),
    }
}

fn clause(object: &Map<String, Json>) -> Result<Clause, JsonError> {
    let name = object
        .get("field")
        .and_then(Json::as_str)
        .ok_or(JsonError::NotANode)?;
    let field = Field::parse(name).ok_or_else(|| JsonError::UnknownField(name.to_owned()))?;

    let op_name = object
        .get("op")
        .and_then(Json::as_str)
        .ok_or(JsonError::NotANode)?;
    let op =
        Operator::parse(op_name).ok_or_else(|| JsonError::UnknownOperator(op_name.to_owned()))?;

    // The same closed table the URL surface honours. A stored view whose field
    // no longer permits its operator is refused rather than executed as
    // something else — `docs/27` constraint 2: rejected at parse time, not at
    // the database.
    if !field.permits(op) {
        return Err(JsonError::OperatorNotPermitted { field, op });
    }

    let value = value_for(op, object.get("value"), field)?;
    Ok(Clause { field, op, value })
}

/// The value an operator can take, from whatever JSON carried it.
fn value_for(op: Operator, raw: Option<&Json>, field: Field) -> Result<Value, JsonError> {
    let shape = || JsonError::ValueShape { field, op };

    // `is_empty` / `is_not_empty` take none. A value beside one is a filter
    // somebody believes is narrower than it is.
    if matches!(op, Operator::IsEmpty | Operator::IsNotEmpty) {
        return match raw {
            None | Some(Json::Null) => Ok(Value::None),
            Some(_) => Err(shape()),
        };
    }

    let raw = raw.ok_or_else(shape)?;

    if op == Operator::Between {
        let bounds = raw.as_array().ok_or_else(shape)?;
        let [low, high] = bounds.as_slice() else {
            return Err(shape());
        };
        let (low, high) = (
            low.as_str().ok_or_else(shape)?,
            high.as_str().ok_or_else(shape)?,
        );
        return Ok(Value::Range(low.to_owned(), high.to_owned()));
    }

    match raw {
        Json::String(text) => Ok(scalar(text)),
        Json::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(item.as_str().ok_or_else(shape)?.to_owned());
            }
            if out.is_empty() {
                // An empty set matches nothing and reads as a mistake. Refusing
                // is the safe direction: a silently-dropped clause returns MORE
                // rows than were asked for.
                return Err(shape());
            }
            Ok(Value::List(out))
        }
        _ => Err(shape()),
    }
}

/// A single value, as a symbol when it looks like one — the URL surface's rule.
fn scalar(raw: &str) -> Value {
    if is_symbolic(raw) {
        Value::Symbol(raw.to_owned())
    } else {
        Value::Literal(raw.to_owned())
    }
}

/// Write an AST back out as the JSON `docs/27` §The AST specifies.
///
/// The inverse of [`from_json`] for every tree [`from_json`] can produce, which
/// is the "AST → JSON → AST is identity" gate in `docs/27` §Acceptance gates.
#[must_use]
pub fn to_json(node: &Node) -> Json {
    match node {
        Node::And(children) => group_json("and", children),
        Node::Or(children) => group_json("or", children),
        Node::Not(inner) => group_json("not", std::slice::from_ref(inner.as_ref())),
        Node::Clause(clause) => {
            let mut object = Map::new();
            object.insert(
                "field".into(),
                Json::String(clause.field.as_str().to_owned()),
            );
            object.insert("op".into(), Json::String(clause.op.as_str().to_owned()));
            // `is_empty` writes no value at all rather than `null`: the absence
            // is the meaning, and a `null` would read as a value somebody set.
            match &clause.value {
                Value::None => {}
                Value::Literal(text) | Value::Symbol(text) => {
                    object.insert("value".into(), Json::String(text.clone()));
                }
                Value::List(items) => {
                    object.insert("value".into(), json_strings(items.iter()));
                }
                // A range writes as a two-element array. `from_json` reads it
                // back as a range because the operator is `between`, which is
                // the only thing that tells it apart from a list.
                Value::Range(low, high) => {
                    object.insert("value".into(), json_strings([low, high]));
                }
            }
            Json::Object(object)
        }
    }
}

fn group_json(op: &str, children: &[Node]) -> Json {
    let mut object = Map::new();
    object.insert("op".into(), Json::String(op.to_owned()));
    object.insert(
        "clauses".into(),
        Json::Array(children.iter().map(to_json).collect()),
    );
    Json::Object(object)
}

fn json_strings<'a>(items: impl IntoIterator<Item = &'a String>) -> Json {
    Json::Array(
        items
            .into_iter()
            .map(|text| Json::String(text.clone()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
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
        let error =
            parse(r#"{ "field": "colour", "op": "eq", "value": "red" }"#).expect_err("unknown");
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
}
