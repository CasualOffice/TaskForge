//! # casual-task-notification
//!
//! Relevance, not coverage (`docs/29-NOTIFICATIONS-AND-DELIVERY.md`).
//!
//! **Owns:** recipient and reason computation, rank resolution so one event yields one notification, preference evaluation, coalescing, and quiet hours.
//!
//! **Must never own:** channel transport. Email and push delivery belong to the worker and `casual-task-infra`.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! # The rule the whole crate serves
//!
//! `docs/29`: "A notification must be something the recipient would act on.
//! Everything else belongs in the activity feed, which is pull, not push."
//!
//! Three modules, three reasons to change:
//!
//! - [`reason`] — the closed, ranked set. Changes when `docs/29`'s reason table
//!   changes. Prevents three notifications for one event.
//! - [`audience`] — candidates to deliveries. Changes when a suppression rule
//!   changes. Prevents notifying somebody about their own action.
//! - [`email`] — what the mail says. Changes when `docs/29` §Email content
//!   changes. Prevents unthreadable subjects and "something changed" bodies.
//!
//! Every one of them is pure: no I/O, no SQL, no clock. Candidate loading is
//! `casual-task-persistence`, and sending is `casual-task-worker`. That split is
//! what lets `docs/29`'s acceptance gates be tested without a database or a
//! relay.
//!
//! # What is implemented, and what is not (C-016)
//!
//! In: the reason set and its ranking, self-action suppression, one
//! notification per recipient at the highest reason, the documented email
//! default for ranks 1–3, and the email's subject and body.
//!
//! Not in, because the schema has nowhere to put them:
//!
//! - **Preferences.** `docs/29` gives a per-user, per-workspace table with a
//!   per-project override. No such table exists, so the documented defaults are
//!   the whole policy rather than its fallback (D-058).
//! - **`SUBSCRIBED`, and unsubscribe.** Both need a subscription row per
//!   `(user, task)`. The reason is declared here so the ranking is complete;
//!   nothing produces it yet (D-058).
//! - **Quiet hours and digests.** Both need the preference table plus a
//!   per-user timezone (D-058).
//!
//! They are named rather than stubbed: a `Reason::Subscribed` nothing emits is
//! visible, and a preferences function that always returns the default is a lie
//! that type-checks.

pub mod audience;
pub mod email;
pub mod reason;

pub use audience::{Candidate, Delivery, resolve};
pub use reason::Reason;
