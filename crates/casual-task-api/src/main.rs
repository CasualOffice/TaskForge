//! # casual-task-api
//!
//! The deployable API binary. The **only** crate that knows about HTTP.
//!
//! Owns: Axum routers, tower middleware (auth context, request id, rate limit,
//! timeout, compression, tracing), DTOs, the generated OpenAPI document, SSE
//! streams, and the error-to-HTTP mapping (`docs/05-API-SPEC.md`).
//!
//! It is also the only crate permitted to call `AuthContext::authenticated`,
//! which is what makes `WorkspaceScope` unforgeable elsewhere
//! (`docs/32-TENANCY-AND-ISOLATION.md`).
//!
//! Phase 0 scaffold — no routes yet. See `docs/14-EXECUTION-TRACKER.md`.

fn main() {
    println!("taskforge api — Phase 0 scaffold, no routes yet (docs/06)");
}
