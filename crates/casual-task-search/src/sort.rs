//! The closed sortable field set (`docs/26` §Cursor pagination).
//!
//! Separate from [`crate::filter::Field`] and deliberately smaller: a field can
//! be filterable without being sortable. Filtering on `title` uses a trigram
//! index; *ordering* by it would be a sort of the whole result set with no
//! index behind it, which is exactly the unbounded cost ADR-011 exists to
//! prevent.
//!
//! `docs/26`: "A sort on anything else is `400 TF-QRY-0002`. This is what makes
//! NFR-5 enforceable rather than aspirational."

use casual_task_model::ErrorCode;
use casual_task_model::error::codes;

/// Every sortable field, each backed by a named index in `docs/26`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortField {
    CreatedAt,
    UpdatedAt,
    DueAt,
    Priority,
    /// The workflow status's position, not the status id — ordering by id would
    /// be ordering by a uuid, which is meaningless to a reader.
    StatusPosition,
    /// The board rank (ADR-013 lexicographic string).
    Position,
    Key,
    /// Full-text relevance. Only meaningful when the filter carries a `q`
    /// clause; ordering by rank without one sorts by a constant.
    Rank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub field: SortField,
    pub direction: Direction,
}

impl SortField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
            Self::DueAt => "due_at",
            Self::Priority => "priority",
            Self::StatusPosition => "status.position",
            Self::Position => "position",
            Self::Key => "key",
            Self::Rank => "rank",
        }
    }

    /// The only way to name a sort field from user input.
    ///
    /// # Errors
    ///
    /// [`codes::QRY_UNSORTABLE_FIELD`] for anything else — including a field
    /// that is perfectly valid to *filter* on. The two sets are different and
    /// the error says which one was violated.
    pub fn parse(name: &str) -> Result<Self, ErrorCode> {
        Ok(match name {
            "created_at" => Self::CreatedAt,
            "updated_at" => Self::UpdatedAt,
            "due_at" => Self::DueAt,
            "priority" => Self::Priority,
            "status.position" => Self::StatusPosition,
            "position" => Self::Position,
            "key" => Self::Key,
            "rank" => Self::Rank,
            _ => return Err(codes::QRY_UNSORTABLE_FIELD),
        })
    }
}

impl Default for Sort {
    /// Newest first. `docs/26`'s worked example orders by `updated_at DESC`,
    /// and a list with no explicit sort is a list somebody expects in that
    /// order.
    fn default() -> Self {
        Self {
            field: SortField::UpdatedAt,
            direction: Direction::Desc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::Field;

    #[test]
    fn every_sortable_field_round_trips() {
        for f in [
            SortField::CreatedAt,
            SortField::UpdatedAt,
            SortField::DueAt,
            SortField::Priority,
            SortField::StatusPosition,
            SortField::Position,
            SortField::Key,
            SortField::Rank,
        ] {
            assert_eq!(SortField::parse(f.as_str()), Ok(f), "{}", f.as_str());
        }
    }

    #[test]
    fn filterable_is_not_sortable() {
        // The two closed sets are different, and conflating them is the mistake
        // worth guarding. `title` filters through a trigram index; ordering by
        // it would sort the whole result set with nothing behind it.
        for filterable_only in ["title", "state", "assignee", "tag", "q", "project"] {
            assert!(
                Field::parse(filterable_only).is_some(),
                "{filterable_only} should be filterable"
            );
            assert_eq!(
                SortField::parse(filterable_only),
                Err(codes::QRY_UNSORTABLE_FIELD),
                "{filterable_only} must not be sortable"
            );
        }
    }

    #[test]
    fn an_unknown_field_is_unsortable_rather_than_defaulted() {
        assert_eq!(SortField::parse("salary"), Err(codes::QRY_UNSORTABLE_FIELD));
        assert_eq!(SortField::parse(""), Err(codes::QRY_UNSORTABLE_FIELD));
    }
}
