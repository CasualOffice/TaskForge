//! The filter AST and its closed field set (`docs/27`).
//!
//! # What makes "nothing scans" enforceable
//!
//! ADR-011 closes the filterable field set so that a filter on an unlisted field
//! is a `400` rather than a slow query. That only holds if the closure is
//! mechanical, so [`Field`] is an enum: a field the design record has not
//! listed, indexed, `EXPLAIN`-asserted and given a UI control cannot be named
//! here, because there is no variant for it.
//!
//! `docs/27` states the rule for adding one — "an entry here, an index in
//! docs/26, an `EXPLAIN` assertion, and a UI control. All four in one change, or
//! none."
//!
//! # Deliberately not an expression language
//!
//! Two node kinds, and no more: a group (`and` / `or` / `not`) and a clause
//! (`field`, `op`, `value`). There are no functions, no arithmetic, and no
//! field-to-field comparison. `docs/27`: "Every one of those would break the
//! index guarantee."
//!
//! # Where the SQL is
//!
//! Not here. `docs/19` puts all SQL in `casual-task-persistence`, and the
//! architecture lint fails the build otherwise. This module produces a
//! *validated* AST; turning one into parameterized SQL through a whitelist is
//! the persistence layer's job, and keeping the two apart is what makes the
//! injection property test meaningful — there is no path from user input to a
//! SQL fragment because this crate cannot emit one.

use casual_task_model::ErrorCode;
use casual_task_model::error::codes;

/// `docs/21` §Query limits.
pub const MAX_CLAUSES: usize = 32;
/// `docs/21` §Query limits.
pub const MAX_DEPTH: usize = 4;

/// Every filterable field. Closed by construction (ADR-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Project,
    Status,
    State,
    Type,
    Priority,
    Assignee,
    Reporter,
    Tag,
    Milestone,
    Environment,
    Parent,
    CreatedAt,
    UpdatedAt,
    DueAt,
    Key,
    Title,
    Q,
    IsBlocked,
    Archived,
}

/// What a field holds, which is what decides its legal operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Id,
    Enum,
    /// `priority` only — a PostgreSQL enum with semantic ordering, so `gte HIGH`
    /// is an index range scan rather than a `CASE` expression (`docs/22`).
    OrderedEnum,
    DateTime,
    Text,
    FullText,
    Boolean,
}

/// Everything an operator can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Eq,
    In,
    NotIn,
    Gt,
    Gte,
    Lt,
    Lte,
    Before,
    After,
    Between,
    IsEmpty,
    IsNotEmpty,
    StartsWith,
    Contains,
    Matches,
}

impl Field {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Status => "status",
            Self::State => "state",
            Self::Type => "type",
            Self::Priority => "priority",
            Self::Assignee => "assignee",
            Self::Reporter => "reporter",
            Self::Tag => "tag",
            Self::Milestone => "milestone",
            Self::Environment => "environment",
            Self::Parent => "parent",
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
            Self::DueAt => "due_at",
            Self::Key => "key",
            Self::Title => "title",
            Self::Q => "q",
            Self::IsBlocked => "is_blocked",
            Self::Archived => "archived",
        }
    }

    /// The only way to name a field from user input.
    ///
    /// Returns `None` rather than a default: `docs/27` constraint 1 makes an
    /// unknown field a `400`, and a lenient parse here would turn ADR-011's
    /// guarantee into a suggestion.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "project" => Self::Project,
            "status" => Self::Status,
            "state" => Self::State,
            "type" => Self::Type,
            "priority" => Self::Priority,
            "assignee" => Self::Assignee,
            "reporter" => Self::Reporter,
            "tag" => Self::Tag,
            "milestone" => Self::Milestone,
            "environment" => Self::Environment,
            "parent" => Self::Parent,
            "created_at" => Self::CreatedAt,
            "updated_at" => Self::UpdatedAt,
            "due_at" => Self::DueAt,
            "key" => Self::Key,
            "title" => Self::Title,
            "q" => Self::Q,
            "is_blocked" => Self::IsBlocked,
            "archived" => Self::Archived,
            _ => return None,
        })
    }

    pub fn field_type(self) -> FieldType {
        match self {
            Self::Project
            | Self::Status
            | Self::Assignee
            | Self::Reporter
            | Self::Tag
            | Self::Milestone
            | Self::Environment
            | Self::Parent => FieldType::Id,
            Self::State | Self::Type => FieldType::Enum,
            Self::Priority => FieldType::OrderedEnum,
            Self::CreatedAt | Self::UpdatedAt | Self::DueAt => FieldType::DateTime,
            Self::Key | Self::Title => FieldType::Text,
            Self::Q => FieldType::FullText,
            Self::IsBlocked | Self::Archived => FieldType::Boolean,
        }
    }

    /// The operators `docs/27` §Fields and their operators permits.
    ///
    /// Per-field rather than per-type, because the table is not uniform by type:
    /// `reporter` takes no `is_empty` (a task always has one), `tag` takes no
    /// `eq` (it is a set), and `key` takes `starts_with` while `title` takes
    /// `contains`. Deriving these from the type would quietly widen the surface.
    pub fn operators(self) -> &'static [Operator] {
        use Operator::*;
        match self {
            Self::Project | Self::Status | Self::State | Self::Type => &[Eq, In, NotIn],
            Self::Priority => &[Eq, In, Gt, Gte, Lt, Lte],
            Self::Assignee => &[Eq, In, IsEmpty, IsNotEmpty],
            Self::Reporter => &[Eq, In],
            Self::Tag => &[In, NotIn, IsEmpty],
            Self::Milestone | Self::Environment => &[Eq, In, IsEmpty],
            Self::Parent => &[Eq, IsEmpty, IsNotEmpty],
            Self::CreatedAt | Self::UpdatedAt => &[Before, After, Between],
            Self::DueAt => &[Before, After, Between, IsEmpty],
            Self::Key => &[Eq, StartsWith],
            Self::Title => &[Contains],
            Self::Q => &[Matches],
            Self::IsBlocked | Self::Archived => &[Eq],
        }
    }

    pub fn permits(self, op: Operator) -> bool {
        self.operators().contains(&op)
    }
}

/// A value, unresolved.
///
/// Symbolic values (`docs/27` §Symbolic values) are kept symbolic through
/// validation and resolved at evaluation, which is what keeps a saved view
/// correct as context changes: `@me` makes "My overdue work" right for every
/// user who opens it, where a hardcoded id would be shareable but wrong.
///
/// Resolution is not this module's job, and the timezone question — `@today`
/// means the **actor's** today, not the server's — belongs with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Literal(String),
    List(Vec<String>),
    /// `@me`, `+7d`, `@start_of_week`, …
    Symbol(String),
    /// `between` takes two.
    Range(String, String),
    /// `is_empty` and `is_not_empty` take none.
    None,
}

/// A leaf: one field, one operator, one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    pub field: Field,
    pub op: Operator,
    pub value: Value,
}

/// The two node kinds, and no others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
    Clause(Clause),
}

/// Why a filter was refused, with the `docs/20` code it maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterError {
    /// ADR-011 — the field is not in the closed set.
    UnknownField(String),
    /// `docs/27` constraint 2 — the operator is not legal for that field.
    OperatorNotPermitted { field: Field, op: Operator },
    /// The value shape does not match the operator.
    MalformedValue { field: Field, op: Operator },
    /// `TF-QRY-0004`.
    TooManyClauses(usize),
    /// `TF-QRY-0005`.
    TooDeep(usize),
}

impl FilterError {
    /// The registered error code (`docs/20`).
    ///
    /// Every variant maps to a code that already exists. `docs/20` is a closed
    /// registry and `docs/15` gates it, so inventing a code here would fail the
    /// build — which is the intended outcome.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::UnknownField(_) => codes::QRY_UNKNOWN_FIELD,
            Self::OperatorNotPermitted { .. } | Self::MalformedValue { .. } => {
                codes::QRY_BAD_OPERATOR
            }
            Self::TooManyClauses(_) => codes::QRY_TOO_MANY_CLAUSES,
            Self::TooDeep(_) => codes::QRY_TOO_DEEP,
        }
    }
}

/// Check a parsed tree against the closed field set and the bounds.
///
/// # Errors
///
/// The first [`FilterError`]. Bounds are checked before shape, so a filter that
/// is both enormous and malformed reports the cheap, structural reason.
pub fn validate(node: &Node) -> Result<(), FilterError> {
    let clauses = count_clauses(node);
    if clauses > MAX_CLAUSES {
        return Err(FilterError::TooManyClauses(clauses));
    }
    let depth = depth_of(node);
    if depth > MAX_DEPTH {
        return Err(FilterError::TooDeep(depth));
    }
    check_shape(node)
}

fn count_clauses(node: &Node) -> usize {
    match node {
        Node::Clause(_) => 1,
        Node::Not(inner) => count_clauses(inner),
        Node::And(children) | Node::Or(children) => children.iter().map(count_clauses).sum(),
    }
}

fn depth_of(node: &Node) -> usize {
    match node {
        Node::Clause(_) => 1,
        Node::Not(inner) => 1 + depth_of(inner),
        Node::And(children) | Node::Or(children) => {
            1 + children.iter().map(depth_of).max().unwrap_or(0)
        }
    }
}

fn check_shape(node: &Node) -> Result<(), FilterError> {
    match node {
        Node::Clause(c) => {
            if !c.field.permits(c.op) {
                return Err(FilterError::OperatorNotPermitted {
                    field: c.field,
                    op: c.op,
                });
            }
            // The value's shape is part of the type check: `between` without two
            // endpoints, or `is_empty` with a value, are the same class of
            // mistake as a wrong operator and are rejected in the same place.
            let ok = match c.op {
                Operator::IsEmpty | Operator::IsNotEmpty => matches!(c.value, Value::None),
                Operator::Between => matches!(c.value, Value::Range(_, _)),
                Operator::In | Operator::NotIn => {
                    matches!(c.value, Value::List(_) | Value::Symbol(_))
                }
                _ => matches!(c.value, Value::Literal(_) | Value::Symbol(_)),
            };
            if !ok {
                return Err(FilterError::MalformedValue {
                    field: c.field,
                    op: c.op,
                });
            }
            Ok(())
        }
        Node::Not(inner) => check_shape(inner),
        Node::And(children) | Node::Or(children) => children.iter().try_for_each(check_shape),
    }
}

#[cfg(test)]
mod tests {
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
}
