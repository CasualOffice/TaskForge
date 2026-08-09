//! The URL surface of the filter grammar (`docs/27` §URL form).
//!
//! ```text
//! ?state=ACTIVE,PLANNED&assignee=@me&due_at=<+7d&priority=>=HIGH&sort=-due_at
//! ```
//!
//! # Why this lives here and not in the HTTP crate
//!
//! `docs/27` §Compilation draws one pipeline with two entry points — a URL
//! string and a JSON tree — meeting at the **same AST** before validation and
//! compilation. Putting the URL reader beside the AST is what keeps that true:
//! there is one place that decides what `<` means, one closed field set, and no
//! second grammar living in a request handler where nothing can test it without
//! a server.
//!
//! It also means the round-trip property `docs/27` §Acceptance gates asks for is
//! testable without HTTP.
//!
//! # The operator is chosen by the field's type, not guessed
//!
//! `<` on a date is `before`; `<` on `priority` is `lt`. Both are spelled `<` in
//! a URL because that is what a human types, and [`Field::field_type`] is what
//! turns the spelling into the right operator. A field that does not permit the
//! resulting operator is a `400` naming the field — never a silently dropped
//! clause, which would return a *larger* result set than asked for.

use crate::filter::{Clause, Field, FieldType, Node, Operator, Value};
use crate::sort::{Direction, Sort, SortField};
use casual_task_model::ErrorCode;
use casual_task_model::error::codes;

/// Query parameters that are not filters.
///
/// Listed once so a caller cannot forget one and have it read as an unknown
/// field — and so adding a pagination parameter later is a change in one place.
pub const RESERVED: &[&str] = &["limit", "cursor", "sort", "include"];

/// A parsed URL query: the filter tree and the requested ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Always an `And` — `docs/27`: "the URL form expresses only flat `AND`".
    pub filter: Node,
    /// Empty when `sort` was absent; the caller applies its own default.
    pub sorts: Vec<Sort>,
}

/// Why a URL query was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    UnknownField(String),
    /// The spelling parsed, but the field does not permit that operator —
    /// `title=>x`, say.
    OperatorNotPermitted {
        field: Field,
        op: Operator,
    },
    /// `x..y` with a missing side.
    MalformedRange(String),
    UnsortableField(String),
}

impl UrlError {
    /// The registry code (`docs/20`) this refusal reports as.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::UnknownField(_) => codes::QRY_UNKNOWN_FIELD,
            Self::OperatorNotPermitted { .. } | Self::MalformedRange(_) => codes::QRY_BAD_OPERATOR,
            Self::UnsortableField(_) => codes::QRY_UNSORTABLE_FIELD,
        }
    }

    /// The field name the caller got wrong, for the error's `details`.
    #[must_use]
    pub fn field(&self) -> String {
        match self {
            Self::UnknownField(name) | Self::UnsortableField(name) => name.clone(),
            Self::OperatorNotPermitted { field, .. } => field.as_str().to_owned(),
            Self::MalformedRange(name) => name.clone(),
        }
    }
}

/// Read a URL query string's parameters into an AST and a sort list.
///
/// Parameters in [`RESERVED`] are skipped. Everything else must name a field in
/// the closed set (`docs/26`: "if a field is not listed here, it is not
/// filterable") or this refuses — a filter on an unlisted field is a `400`, not
/// a slow query.
///
/// # Errors
///
/// [`UrlError`] for an unknown field, an operator the field does not permit, a
/// malformed range, or an unsortable sort key.
pub fn parse<'a, I>(params: I) -> Result<Query, UrlError>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut clauses = Vec::new();
    let mut sorts = Vec::new();

    for (name, raw) in params {
        if name == "sort" {
            sorts = parse_sort(raw)?;
            continue;
        }
        if RESERVED.contains(&name) {
            continue;
        }
        let field = Field::parse(name).ok_or_else(|| UrlError::UnknownField(name.to_owned()))?;
        clauses.push(Node::Clause(clause_of(field, raw)?));
    }

    Ok(Query {
        filter: Node::And(clauses),
        sorts,
    })
}

/// One `field=value` pair, as the table in `docs/27` §URL form defines it.
fn clause_of(field: Field, raw: &str) -> Result<Clause, UrlError> {
    let permit = |op: Operator, value: Value| -> Result<Clause, UrlError> {
        if field.permits(op) {
            Ok(Clause { field, op, value })
        } else {
            Err(UrlError::OperatorNotPermitted { field, op })
        }
    };

    // `field=` — the empty value is how a URL says "unset". It is checked first
    // because every other rule would read it as an empty literal.
    if raw.is_empty() {
        return permit(Operator::IsEmpty, Value::None);
    }

    // `field=x..y`
    if let Some((low, high)) = raw.split_once("..") {
        if low.is_empty() || high.is_empty() {
            return Err(UrlError::MalformedRange(field.as_str().to_owned()));
        }
        return permit(
            Operator::Between,
            Value::Range(low.to_owned(), high.to_owned()),
        );
    }

    // `field=!a` / `field=!a,b`
    if let Some(rest) = raw.strip_prefix('!') {
        return permit(Operator::NotIn, list(rest));
    }

    // Ordering prefixes. `>=` and `<=` are tested before `>` and `<` so the
    // longer spelling is not read as the shorter one plus a stray `=`.
    for (prefix, ordered, dated) in [
        (">=", Operator::Gte, Operator::Gte),
        ("<=", Operator::Lte, Operator::Lte),
        (">", Operator::Gt, Operator::After),
        ("<", Operator::Lt, Operator::Before),
    ] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            // The same spelling means different operators on different types:
            // `<` on a date is `before`, on `priority` it is `lt`.
            let op = if field.field_type() == FieldType::DateTime {
                dated
            } else {
                ordered
            };
            return permit(op, scalar(rest));
        }
    }

    // `field=a,b` — a comma is what makes it a set.
    if raw.contains(',') {
        return permit(Operator::In, list(raw));
    }

    // A bare value takes the field type's natural operator.
    let op = match field.field_type() {
        FieldType::FullText => Operator::Matches,
        FieldType::Text if field == Field::Title => Operator::Contains,
        _ => Operator::Eq,
    };
    permit(op, scalar(raw))
}

/// A single value, as a symbol when it looks like one.
///
/// `@me`, `@today`, `+7d`, `-3mo` are resolved later against the actor and
/// their timezone (`crate::resolve`); everything else is a literal. Deciding it
/// here rather than in the resolver keeps `Value::Symbol` meaning "needs
/// resolution" throughout.
fn scalar(raw: &str) -> Value {
    if is_symbol(raw) {
        Value::Symbol(raw.to_owned())
    } else {
        Value::Literal(raw.to_owned())
    }
}

/// A comma-separated set. A set of one symbol stays a symbol — `@my_teams`
/// expands to a list during resolution, and wrapping it in a one-element list
/// here would hide that.
fn list(raw: &str) -> Value {
    if is_symbol(raw) && !raw.contains(',') {
        return Value::Symbol(raw.to_owned());
    }
    Value::List(raw.split(',').map(ToOwned::to_owned).collect())
}

/// Whether a value is a symbol the resolver understands.
///
/// Whether a value is a symbol the resolver understands.
///
/// Delegates to [`crate::filter::is_symbolic`], which the JSON surface uses too:
/// `docs/27` §Compilation has one AST with two entry points, and two copies of
/// this rule would let the same saved view mean different things depending on
/// which door it came through.
fn is_symbol(raw: &str) -> bool {
    crate::filter::is_symbolic(raw)
}

/// `sort=-due_at,key` — leading `-` is descending.
fn parse_sort(raw: &str) -> Result<Vec<Sort>, UrlError> {
    raw.split(',')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (direction, name) = part
                .strip_prefix('-')
                .map_or_else(|| (Direction::Asc, part), |rest| (Direction::Desc, rest));
            SortField::parse(name)
                .map(|field| Sort { field, direction })
                .map_err(|_| UrlError::UnsortableField(name.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str, value: &str) -> Result<Clause, UrlError> {
        let query = parse([(name, value)])?;
        match query.filter {
            Node::And(mut clauses) if clauses.len() == 1 => match clauses.remove(0) {
                Node::Clause(clause) => Ok(clause),
                other => panic!("expected a clause, got {other:?}"),
            },
            other => panic!("expected one clause, got {other:?}"),
        }
    }

    #[test]
    fn the_documented_example_parses() {
        // docs/27 §URL form, character for character.
        let query = parse([
            ("state", "ACTIVE,PLANNED"),
            ("assignee", "@me"),
            ("due_at", "<+7d"),
            ("priority", ">=HIGH"),
            ("sort", "-due_at"),
        ])
        .expect("the documented example must parse");

        let Node::And(clauses) = &query.filter else {
            panic!("the URL form is flat AND");
        };
        assert_eq!(clauses.len(), 4);
        assert_eq!(
            query.sorts,
            vec![Sort {
                field: SortField::DueAt,
                direction: Direction::Desc
            }]
        );
    }

    #[test]
    fn every_row_of_the_documented_table_is_implemented() {
        // field=a,b → in
        assert_eq!(
            one("state", "ACTIVE,PLANNED").expect("in"),
            Clause {
                field: Field::State,
                op: Operator::In,
                value: Value::List(vec!["ACTIVE".into(), "PLANNED".into()]),
            }
        );
        // field=!a → not_in
        assert_eq!(one("state", "!DONE").expect("not_in").op, Operator::NotIn);
        // field=<x / field=>x on a DATE → before / after
        assert_eq!(
            one("due_at", "<2026-01-01").expect("before").op,
            Operator::Before
        );
        assert_eq!(
            one("due_at", ">2026-01-01").expect("after").op,
            Operator::After
        );
        // the same spellings on an ORDERED ENUM → lt / gt
        assert_eq!(one("priority", ">HIGH").expect("gt").op, Operator::Gt);
        assert_eq!(one("priority", "<HIGH").expect("lt").op, Operator::Lt);
        // field=>=x → gte
        assert_eq!(one("priority", ">=HIGH").expect("gte").op, Operator::Gte);
        // field=x..y → between
        assert_eq!(
            one("due_at", "2026-01-01..2026-02-01")
                .expect("between")
                .value,
            Value::Range("2026-01-01".into(), "2026-02-01".into())
        );
        // field= → is_empty
        assert_eq!(
            one("assignee", "").expect("is_empty"),
            Clause {
                field: Field::Assignee,
                op: Operator::IsEmpty,
                value: Value::None,
            }
        );
    }

    #[test]
    fn a_longer_operator_is_not_read_as_a_shorter_one() {
        // `>=HIGH` read as `>` plus a literal `=HIGH` would compare priority
        // against a value no enum has, and return nothing rather than erroring.
        assert_eq!(
            one("priority", ">=HIGH").expect("gte").value,
            Value::Literal("HIGH".into())
        );
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        // docs/26: "if a field is not listed here, it is not filterable". A
        // dropped clause would return MORE rows than the user asked for, which
        // is the dangerous direction to fail.
        let error = parse([("colour", "red")]).expect_err("unknown field");
        assert_eq!(error, UrlError::UnknownField("colour".into()));
        assert_eq!(error.code(), codes::QRY_UNKNOWN_FIELD);
    }

    #[test]
    fn an_operator_the_field_forbids_is_refused() {
        // `title` permits only `contains`. Accepting `>` would compile to a
        // string comparison nobody asked for.
        let error = parse([("title", ">abc")]).expect_err("not permitted");
        assert_eq!(error.code(), codes::QRY_BAD_OPERATOR);
    }

    #[test]
    fn reserved_parameters_are_not_filters() {
        let query = parse([
            ("limit", "50"),
            ("cursor", "abc"),
            ("include", "count"),
            ("q", "payment retry"),
        ])
        .expect("reserved parameters are skipped");
        let Node::And(clauses) = &query.filter else {
            panic!("flat AND");
        };
        assert_eq!(clauses.len(), 1, "only q is a filter");
    }

    #[test]
    fn free_text_and_title_take_their_natural_operators() {
        assert_eq!(one("q", "payment retry").expect("q").op, Operator::Matches);
        assert_eq!(one("title", "retry").expect("title").op, Operator::Contains);
    }

    #[test]
    fn symbols_survive_to_the_resolver() {
        // The resolver needs to see `@me` as a Symbol. A Literal would reach
        // the database as the six characters `"@me"`.
        assert_eq!(
            one("assignee", "@me").expect("symbol").value,
            Value::Symbol("@me".into())
        );
        assert_eq!(
            one("due_at", "<+7d").expect("relative").value,
            Value::Symbol("+7d".into())
        );
        assert_eq!(
            one("assignee", "@my_teams").expect("list symbol").value,
            Value::Symbol("@my_teams".into()),
            "a set-valued symbol must stay a symbol so resolution can expand it"
        );
    }

    #[test]
    fn an_unsigned_relative_offset_is_not_a_symbol() {
        // `resolve` requires the sign. Treating `7d` as a symbol here would
        // produce UnknownSymbol later; leaving it a literal makes it fail as
        // the malformed date it is.
        assert_eq!(
            one("due_at", "<7d").expect("literal").value,
            Value::Literal("7d".into())
        );
    }

    #[test]
    fn a_malformed_range_is_refused() {
        for raw in ["..2026-01-01", "2026-01-01..", ".."] {
            assert!(parse([("due_at", raw)]).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn sort_direction_and_unsortable_fields() {
        assert_eq!(
            parse_sort("-due_at,key").expect("two keys"),
            vec![
                Sort {
                    field: SortField::DueAt,
                    direction: Direction::Desc
                },
                Sort {
                    field: SortField::Key,
                    direction: Direction::Asc
                },
            ]
        );
        // docs/26: a sort on anything else is TF-QRY-0002.
        let error = parse_sort("colour").expect_err("unsortable");
        assert_eq!(error.code(), codes::QRY_UNSORTABLE_FIELD);
    }
}
