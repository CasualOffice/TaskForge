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

pub mod config;
pub mod error;
pub mod server;

pub use config::{Config, ConfigError};
pub use error::ApiError;
pub use server::{AppState, router, serve};
