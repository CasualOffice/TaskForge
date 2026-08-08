//! # casual-task-identity
//!
//! Identity and access primitives.
//!
//! **Owns:** users, workspace membership, teams, sessions, service accounts, and API tokens (`docs/40-IDENTITY-AUTH-AND-SESSION.md`).
//!
//! **Must never own:** permission decisions — those belong to `casual-task-authz`. Authentication answers *who*; authorization answers *what may they do*.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!

pub mod credential;
pub mod mfa;
pub mod password;

pub use credential::{Invalid, Minted};
pub use mfa::{RecoveryCode, Totp};
pub use password::PasswordError;
