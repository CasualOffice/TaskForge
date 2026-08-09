//! `/api/v1/workspaces` and `/api/v1/teams` (C-002).
//!
//! # Why this is a directory
//!
//! One 1,390-line file held eleven handlers covering three different subjects.
//! The split is by the question each answers, so a change to how membership
//! works cannot accidentally land in the code that creates a workspace:
//!
//! - [`wire`] — the request and response shapes (`docs/05`).
//! - [`lifecycle`] — a workspace coming into and out of existence, including
//!   the owner grant that is part of its creation (D-054).
//! - [`members`] — who is visible in a workspace. Not who may do what.
//! - [`teams`] — teams, which exist because a grant can be assigned to one.

pub mod lifecycle;
pub mod members;
pub mod support;
pub mod teams;
pub mod wire;

pub use lifecycle::{create, list, read, update};
pub use members::{add_member, list_members, remove_member};
pub use teams::{add_team_member, create_team, list_team_members, list_teams, remove_team_member};

pub(crate) use support::*;
pub(crate) use wire::*;
