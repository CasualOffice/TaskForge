//! Export: a task query, as a file (C-021, `docs/38` §Export).
//!
//! # The failure this module exists to prevent
//!
//! Two, and they are unrelated except that both are specific to exports.
//!
//! **A synchronous export.** `docs/38`: "Anything above 1,000 rows is
//! asynchronous." A request that streams 200,000 rows holds an HTTP connection
//! and a database transaction for minutes; the client's only recovery from a
//! timeout is to start again, which is how one impatient user becomes four
//! concurrent full-table reads. So an export is a row in `export_job`, a worker
//! that runs it, and an artefact in object storage.
//!
//! **A stale grant.** Authority is resolved once per request everywhere else in
//! this product, and everywhere else a request is over in milliseconds. An
//! export outlives its own authorization: `docs/38` requires permissions to be
//! "evaluated per batch, not once at the start", and [`runner`] re-resolves the
//! actor's accessible project set before every page rather than compiling the
//! filter once and trusting it to the end.
//!
//! # The split
//!
//! | Module | Changes when |
//! | --- | --- |
//! | [`csv`] | RFC 4180, or the formula-injection defence, does |
//! | [`jsonl`] | the line-delimited JSON shape does |
//! | [`runner`] | how a job is claimed, batched, or audited does |
//!
//! [`csv`] is separate from [`jsonl`] rather than sharing a `Serializer` trait
//! because they do not share a problem: one is defending against a spreadsheet
//! executing its input, and the other is writing JSON. A shared abstraction
//! would have to be wide enough for both and would earn nothing.

pub mod csv;
pub mod jsonl;
pub mod runner;

/// What a job asked for. `docs/38` §Formats.
///
/// XLSX is named there too, generated through OpenCalc. It is not here: that is
/// a dependency edge and a second writer, and neither is needed to make export
/// useful. Absent rather than stubbed, so nothing reports a format it cannot
/// produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Spreadsheets — "the 90% case". Formula-escaped; see [`csv`].
    Csv,
    /// Pipelines and scripts. Not formula-escaped, deliberately: a JSON parser
    /// has never executed a cell, and adding apostrophes would corrupt the data
    /// for the consumer this format exists for.
    Jsonl,
}

impl Format {
    /// Parse the wire value. Unknown formats are rejected at the edge rather
    /// than defaulted, so a typo is a `400` and not a surprising file.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "csv" => Some(Self::Csv),
            "jsonl" => Some(Self::Jsonl),
            _ => None,
        }
    }

    /// The stored spelling, and what `GET /exports/{id}` reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Jsonl => "jsonl",
        }
    }

    /// What a browser should be told the artefact is.
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            // `charset=utf-8` alongside the BOM: belt and braces, because the
            // two are read by different things — the header by browsers, the
            // BOM by Excel.
            Self::Csv => "text/csv; charset=utf-8",
            Self::Jsonl => "application/x-ndjson",
        }
    }

    /// The artefact's file extension.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Jsonl => "jsonl",
        }
    }
}

/// The columns an export may contain, in the order `docs/38` lists them as "the
/// same closed field set as everything else".
///
/// A closed enum rather than free strings: a column name that reached SQL from a
/// request would be the injection hole the filter grammar exists to close, and a
/// column that named a field the actor cannot see would be an authorization hole
/// wearing a projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Key,
    Title,
    Type,
    Priority,
    State,
    Reporter,
    DueAt,
    CreatedAt,
    UpdatedAt,
}

impl Column {
    /// Every column, which is also the default projection.
    pub const ALL: &'static [Self] = &[
        Self::Key,
        Self::Title,
        Self::Type,
        Self::Priority,
        Self::State,
        Self::Reporter,
        Self::DueAt,
        Self::CreatedAt,
        Self::UpdatedAt,
    ];

    /// Parse a requested column name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|column| column.as_str() == value)
    }

    /// The header cell, and the JSONL object key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Title => "title",
            Self::Type => "type",
            Self::Priority => "priority",
            Self::State => "state",
            Self::Reporter => "reporter_id",
            Self::DueAt => "due_at",
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_format_is_refused_rather_than_defaulted() {
        // A typo that silently produced a CSV would hand someone the wrong file
        // and no reason to doubt it.
        assert_eq!(Format::parse("csv"), Some(Format::Csv));
        assert_eq!(Format::parse("jsonl"), Some(Format::Jsonl));
        assert_eq!(Format::parse("xlsx"), None, "XLSX is not built yet");
        assert_eq!(Format::parse("CSV"), None);
        assert_eq!(Format::parse(""), None);
    }

    #[test]
    fn an_unknown_column_is_refused() {
        assert_eq!(Column::parse("title"), Some(Column::Title));
        // The shape of an injection attempt, and of a typo. Both are refused by
        // the same closed set.
        assert_eq!(Column::parse("t.title, (SELECT 1)"), None);
        assert_eq!(Column::parse("description"), None);
    }

    #[test]
    fn every_column_round_trips_through_its_wire_name() {
        for column in Column::ALL {
            assert_eq!(
                Column::parse(column.as_str()),
                Some(*column),
                "{} does not parse back to itself",
                column.as_str()
            );
        }
    }

    #[test]
    fn the_formats_do_not_share_a_content_type_or_extension() {
        assert_ne!(Format::Csv.content_type(), Format::Jsonl.content_type());
        assert_ne!(Format::Csv.extension(), Format::Jsonl.extension());
    }
}
