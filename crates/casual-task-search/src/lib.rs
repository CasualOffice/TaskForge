//! # casual-task-search
//!
//! Search projection and query construction (`docs/26-SEARCH-INDEXING-AND-QUERY.md`).
//!
//! **Owns:** search document construction, ranking, and the filter-to-SQL compiler that injects the permission predicate rather than accepting it from a caller.
//!
//! **Must never own:** the authorization decision itself. It requires an authorized project set; it never computes one. Kept separate from `-persistence` because this is the seam an external engine would replace (ADR-014).
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! The filter AST and its closed field set are implemented (C-012). The search
//! projection and cursor pagination are not. See `docs/14-EXECUTION-TRACKER.md`.

pub mod filter;

pub use filter::{
    Clause, Field, FieldType, FilterError, MAX_CLAUSES, MAX_DEPTH, Node, Operator, Value, validate,
};
