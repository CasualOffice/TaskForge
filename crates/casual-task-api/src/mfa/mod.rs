//! Multi-factor authentication (C-001, `docs/40` §MFA).
//!
//! # Why this is a directory
//!
//! `AGENTS.md` §Module size and shape: split by **reason to change**, not by
//! size. MFA has four of them, and they move independently:
//!
//! - [`wire`] — the request and response shapes. Changes when `docs/05` does.
//! - [`enrol`] — a factor coming into and out of existence. Changes when the
//!   enrolment ceremony does.
//! - [`challenge`] — proving a factor *now*: the step-up, the replay refusal,
//!   and recovery-code redemption. Changes when `docs/40`'s verification rules
//!   do.
//! - [`policy`] — whether a workspace demands MFA, and the one function that
//!   answers "does this session satisfy it". Changes when `docs/04`/`docs/40`
//!   authority does.
//!
//! [`policy`] carries the weight the `guard` split carries elsewhere. The
//! step-up decision is made in **one** place and consumed by workspace
//! resolution, so a second entry point cannot grow a slightly different idea of
//! what counts as satisfied — which is how one route ends up demanding a factor
//! and its neighbour does not.
//!
//! # The two rules everything here is arranged around
//!
//! **An unconfirmed factor never satisfies MFA.** `docs/40` and migration 0016
//! both say it: a user who scanned the QR code and closed the tab must not be
//! locked out by a factor they do not have. The rule lives in the repository's
//! `WHERE` clause, not in these handlers, so no handler can forget it.
//!
//! **The secret never reaches a log.** It is the one recoverable plaintext in
//! the schema. It is returned exactly once, by [`enrol::begin`], and everywhere
//! else it is wrapped in `casual_task_observability::Redacted` so that
//! `Debug`, `Display` and `Serialize` all print `<redacted>` — the leak the
//! type exists to make impossible rather than merely discouraged.

pub mod challenge;
pub mod enrol;
pub mod policy;
pub mod wire;

pub use challenge::{step_up, verify_recovery_code};
pub use enrol::{begin, confirm, disable, status};
pub use policy::set_requirement;

pub use policy::{step_up_refusal, step_up_required};
pub use wire::MfaStatus;
