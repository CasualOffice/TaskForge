//! `/api/v1/workflows` — reading a workflow, and authoring one (`docs/23`).
//!
//! # The split, and what each part changes for
//!
//! | Module | Changes when |
//! | --- | --- |
//! | [`wire`] | the API contract does (`docs/05`) |
//! | [`guard`] | who may author a workflow does (`docs/04`) |
//! | [`audit`] | what a workflow edit records does (`docs/25`) |
//! | [`read`](mod@read) | the board's view of a workflow does |
//! | [`statuses`] | the rules for editing a status do (`docs/23`) |
//! | [`migrate`] | the rules for moving in-flight work off one do |
//! | [`transitions`] | the rules for editing an edge do |
//!
//! [`guard`] carries the weight. Six authoring handlers all need the same
//! answer — "may this actor reshape this workflow, and is the workflow's
//! version still the one they were looking at" — and the way that goes wrong is
//! for one of the six to assemble it slightly differently, which is how the
//! endpoint beside it ends up more permissive.
//!
//! # Why authoring is one aggregate under one version
//!
//! Statuses and transitions have no version column, and `docs/24` needs a
//! conditional write for anything an admin might race another admin on. The
//! workflow's `version` is that guard for all of it: every authoring call
//! carries `If-Match` against it and bumps it. Versioning the rows individually
//! would let "delete Blocked, migrating to In Progress" and "delete In
//! Progress" both succeed, because they touch different rows — and leave tasks
//! pointing at a status that no longer exists.
//!
//! # What is deliberately absent
//!
//! Changing a project's workflow. `docs/23` calls it "the heaviest operation",
//! requires an explicit status-by-status mapping, and specifies it as a
//! background job — the same job a >10,000-task status migration needs, which
//! does not exist yet (**D-063**). It is left unbuilt rather than approximated,
//! because a half-built version of it would move real work.

pub mod audit;
pub mod guard;
pub mod migrate;
pub mod read;
pub mod statuses;
pub mod transitions;
pub mod wire;

pub use migrate::delete_status;
pub use read::read;
pub use statuses::{create_status, list_statuses, reorder_statuses, update_status};
pub use transitions::{create_transition, delete_transition, update_transition};
pub use wire::{StatusView, TransitionView, WorkflowView};
