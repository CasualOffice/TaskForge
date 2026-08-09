//! # casual-task-api
//!
//! The deployable API binary, and the **only** crate that knows about HTTP.
//!
//! Owns: Axum routers, tower middleware (request id, metrics, and later auth
//! context and rate limiting), DTOs, the generated OpenAPI document, SSE
//! streams, and the error-to-HTTP mapping (`docs/05-API-SPEC.md`).
//!
//! It is also the only crate permitted to call `AuthContext::authenticated`,
//! which is what makes `WorkspaceScope` unforgeable elsewhere
//! (`docs/32-TENANCY-AND-ISOLATION.md`).
//!
//! # Why this is a library as well as a binary
//!
//! The behaviour worth testing here is a *request reaching a response*: the
//! error envelope, the request-id echo, the readiness code under a dead
//! database. Those need a router, and a router cannot be reached from inside
//! `main`.

pub mod activity;
pub mod attachments;
pub mod auth;
pub mod comments;
pub mod config;
pub mod context;
pub mod csrf;
pub mod dependencies;
pub mod error;
pub mod etag;
pub mod exports;
pub mod invitations;
pub mod json;
pub mod middleware;
pub mod notifications;
pub mod password_reset;
pub mod permissions;
pub mod projects;
pub mod rate_limit;
pub mod server;
pub mod sse;
pub mod tasks;
pub mod unit;
pub mod wire;
pub mod workflows;
pub mod workspaces;

pub use config::{Config, ConfigError};
pub use error::ApiError;
pub use middleware::{Authenticated, WorkspaceMember};
pub use server::{AppState, router, serve};
