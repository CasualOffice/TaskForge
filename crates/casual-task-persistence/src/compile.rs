//! Compiling a validated filter to parameterized SQL (`docs/27` §Compilation).
//!
//! # The one rule
//!
//! "The compiler emits only `$1`-style parameters and identifiers from a static
//! map. There is no string interpolation of user data anywhere in the path."
//!
//! Every user-supplied value becomes a [`Param`] and is referenced by position.
//! The only text that reaches the SQL string is a column name from the private
//! static identifier map, an operator from a fixed `match`, and digits. A
//! hostile value has nowhere to go: `Value` holds `String`s, and no code path
//! writes one into the output.
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

use casual_task_model::Cursor;
use casual_task_model::{ProjectId, WorkspaceId};
use casual_task_search::filter::{Field, Node, Operator, Value};
use casual_task_search::sort::{Direction, Sort, SortField};

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

/// How a page is asked for: what to sort by, where to resume, and how many.
///
/// `docs/26` bans `OFFSET` — it scans, and it duplicates or skips rows under
/// concurrent writes, "both of which a live board guarantees". A page is
/// therefore always a keyset: a predicate on `(sort_key, id)` against the
/// previous page's last row.
#[derive(Debug, Clone)]
pub struct Page {
    pub sort: Sort,
    /// `None` for the first page.
    pub after: Option<Cursor>,
    /// Rows wanted. One more is always fetched, to detect a next page without
    /// a second count query.
    pub limit: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            sort: Sort::default(),
            after: None,
            limit: 50,
        }
    }
}

/// The column a sort field orders by. The static identifier map for sorting.
fn sort_column(field: SortField) -> &'static str {
    match field {
        SortField::CreatedAt => "t.created_at",
        SortField::UpdatedAt => "t.updated_at",
        SortField::DueAt => "t.due_at",
        SortField::Priority => "t.priority",
        SortField::StatusPosition => "ws.position",
        SortField::Position => "t.position",
        SortField::Key => "t.number",
        SortField::Rank => "rank",
    }
}

/// The PostgreSQL type a bound value must be cast to.
///
/// The cast goes on the **parameter**, never the column. `t.state = $3::task_state`
/// uses the index on `state`; `t.state::text = $3` does not, and would turn
/// every filtered list into a sequential scan — the thing NFR-5 forbids and the
/// `EXPLAIN` gate exists to catch.
fn cast_for(field: Field) -> &'static str {
    match field {
        Field::Project
        | Field::Status
        | Field::Assignee
        | Field::Reporter
        | Field::Tag
        | Field::Milestone
        | Field::Environment
        | Field::Team
        | Field::Parent => "uuid",
        Field::State => "task_state",
        Field::Type => "task_type",
        Field::Priority => "task_priority",
        Field::CreatedAt | Field::UpdatedAt | Field::DueAt => "timestamptz",
        // Both are BOOLEAN in the grammar (`casual_task_search::FieldType`)
        // even though `archived` is stored as a nullable timestamp: the filter
        // asks "is it archived", and the comparison is against
        // `archived_at IS NOT NULL`.
        Field::IsBlocked | Field::Archived => "boolean",
        // Compared as text; no cast needed, and `q` is handled structurally.
        Field::Title | Field::Q | Field::Key => "",
    }
}

/// `$3::uuid`, or `$3` when the field needs no cast.
fn cast(placeholder: &str, field: Field, list: bool) -> String {
    let ty = cast_for(field);
    if ty.is_empty() {
        placeholder.to_owned()
    } else if list {
        format!("{placeholder}::{ty}[]")
    } else {
        format!("{placeholder}::{ty}")
    }
}

/// SQL plus the parameters it references, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub sql: String,
    pub params: Vec<Param>,
}

/// The ranking expression (`docs/26` §Weighting).
///
/// `ts_rank_cd` — cover density — rather than `ts_rank`: it accounts for how
/// close the matched lexemes are to each other, so a title containing the whole
/// phrase outranks a description that happens to mention both words pages
/// apart.
const RANK: &str = "ts_rank_cd(s.document, q)";

/// How a typed term becomes the bound argument to `to_tsquery`.
///
/// # Why not `plainto_tsquery` alone
///
/// It was `plainto_tsquery` alone, and that meant **no result until a whole
/// word was typed**. `backu` found nothing; `backup` found the task. Every
/// keystroke before the last was an empty list, which is the single thing that
/// made search feel broken — a search box that answers only complete words is
/// a search box you have to already know the answer to use.
///
/// `docs/26` §index inventory assigns prefix matching to `task_search_trgm`,
/// which is created by migration 0009, filled on every write, and read by
/// nothing. **D-069** ruled that the trigram path stays unwired until its plan
/// shape is measured — `compile_search`'s own notes explain why an `OR` across
/// two indexes is not a free change under D-043 — and that prefix is served
/// meanwhile by a `:*` on the final token, which the existing `task_search_gin`
/// already answers and which leaves the plan alone.
///
/// # Only the last token
///
/// The last token is the one being typed; the ones before it are finished
/// words. `restore backu` becomes `restore & backu:*`, which still finds
/// "Backup restore drill". Making every token a prefix would match far more
/// than anyone asked for.
///
/// # Why the term is rebuilt rather than passed through
///
/// `to_tsquery` parses its argument as tsquery *syntax*, so `&`, `|`, `!`,
/// `(`, `)` and `:` in a person's typing are operators rather than text. The
/// term is therefore reduced to alphanumerics and `-` — everything else is
/// dropped, not escaped — and the operators are supplied by this function
/// alone. Hyphens are kept because `OPS-1` is a task key and dropping the
/// hyphen would break the search people use most.
///
/// Measured against PostgreSQL 16 rather than assumed: hyphen-only and
/// stopword-only inputs do not raise — they produce an empty tsquery, which
/// matches nothing — so the only real hazard is an operator reaching the
/// parser, and none can.
///
/// When nothing survives the filter there is no prefix to add, so an empty
/// query is bound. PostgreSQL 16 turns that into an empty `tsquery`; binding the
/// original punctuation would put operators back into the syntax parser.
fn tsquery_of(term: &str) -> String {
    let tokens: Vec<String> = term
        .split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-')
                .collect::<String>()
        })
        // A token of punctuation alone contributes no lexeme. Requiring one
        // alphanumeric rather than merely non-empty is what stops `-` becoming
        // the token `-:*`.
        .filter(|token| token.chars().any(char::is_alphanumeric))
        .collect();

    if tokens.is_empty() {
        return String::new();
    }
    format!("{}:*", tokens.join(" & "))
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
        Field::Team => "t.team_id",
        Field::Parent => "t.parent_id",
        Field::CreatedAt => "t.created_at",
        Field::UpdatedAt => "t.updated_at",
        Field::DueAt => "t.due_at",
        Field::Title => "t.title",
        Field::Archived => "t.archived_at",
        // Handled structurally rather than as a column comparison.
        Field::Assignee | Field::Tag | Field::Q | Field::IsBlocked | Field::Key => "",
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
    page: &Page,
) -> Compiled {
    let mut params: Vec<Param> = Vec::new();

    // Injected first, always. Not conditional, not caller-supplied.
    params.push(Param::Workspace(workspace));
    params.push(Param::Projects(authorized.as_slice().to_vec()));

    // A `q` clause changes the SHAPE of the query, not just its predicate:
    // `docs/26` §Permission filtering joins the projection and ranks in the
    // select list, because a rank cannot be computed from a subquery the
    // planner is free to turn into a semi-join. Hoisted here so the clause
    // emitter never sees it.
    if let Some(term) = full_text_term(filter) {
        return compile_search(filter, term, page, params);
    }

    let predicate = emit(filter, &mut params);
    let col = sort_column(page.sort.field);
    let keyset = keyset_predicate(page, &mut params, col);
    let dir = match page.sort.direction {
        Direction::Asc => "ASC",
        Direction::Desc => "DESC",
    };
    // The id tiebreaker is mandatory and orders the same way as the key.
    // `docs/26`: without it, ties in `updated_at` — which happen constantly on
    // bulk operations — make the cursor non-deterministic, so a page can repeat
    // or skip a row.
    //
    // LIMIT is n + 1: the extra row is how "has next page" is answered without
    // a COUNT over the whole result set.
    let limit = page.limit.saturating_add(1);

    // The projection is the repository's, not `t.*`: `type`, `priority` and
    // `state` are PostgreSQL enums, and a `SELECT *` hands them back as enums
    // that no `String` decoder accepts. One projection, defined once, used by
    // the compiler and by `crate::task::read_visible` alike.
    let columns = crate::task::COLUMNS;
    let sql = format!(
        "SELECT {columns} FROM task t \
         WHERE t.workspace_id = $1 \
           AND t.project_id = ANY($2) \
           AND t.deleted_at IS NULL \
           AND ({predicate}){keyset} \
         ORDER BY {col} {dir}, t.id {dir} \
         LIMIT {limit}"
    );
    Compiled { sql, params }
}

include!("compile_reports.rs");
include!("compile_predicates.rs");
#[cfg(test)]
#[path = "compile_tests.rs"]
mod tests;
