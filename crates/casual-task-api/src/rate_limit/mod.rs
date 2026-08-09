//! Rate limiting at the edge (C-020, `docs/21` §Rate limits).
//!
//! # The failure this module prevents
//!
//! Work done on behalf of a caller who should already have been refused.
//! `docs/21` §Enforcement order puts the bucket check at step 4 — after the one
//! indexed read that identifies the caller, and before the body is parsed, the
//! permission resolver runs, or any handler touches a tenant row.
//!
//! Three files, three reasons to change:
//!
//! - [`class`] — the published numbers and which class a request is in.
//!   Changes when `docs/21`'s table changes.
//! - [`meter`] — the GCRA bucket, the bounded map, and the D-040 overflow
//!   policy. Changes when the algorithm changes.
//! - [`layer`] — the two middlewares and the `RateLimit-*` headers. Changes
//!   when routing or the wire behaviour changes.
//!
//! # Two limiters, because there are two kinds of caller
//!
//! The auth class runs *before* anybody is authenticated — that is what it is
//! for — so it keys on the client address. Everything else keys per
//! `(workspace, actor)`, which is what `docs/21` specifies and what makes
//! "exhausting one workspace's bucket does not affect another's" true.

pub mod class;
pub mod layer;
pub mod meter;

pub use class::{AUTH, BULK, Class, INVITE, LIMITED_ROUTES, READ, SEARCH, WRITE, classify};
pub use layer::{
    PrincipalLimits, PrincipalState, RateLimitState, principal_rate_limit, rate_limit,
};
pub use meter::{Decision, MAX_TRACKED_KEYS, RateLimiter, SWEEP_INTERVAL, Scope, client_ip};
