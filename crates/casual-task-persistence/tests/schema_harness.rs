//! The PostgreSQL test harness (F-005), and the first assertions that use it.
//!
//! # Why this exists when `scripts/verify-schema.sh` already passes
//!
//! That script is the gate, and it stays the gate. What it cannot do is hand a
//! *connection* to a Rust test. Every repository written in Phase 1 needs a
//! real PostgreSQL with the migrations applied — `sqlx` is compile-time checked
//! against a live schema, and the tenant-isolation suites in `docs/15` are
//! per-endpoint Rust tests, not shell assertions. Building that seam now means
//! C-001's first repository test starts with a database instead of inventing
//! one.
//!
//! # Why it is `#[ignore]` by default
//!
//! It needs a working Docker daemon and pulls a ~250 MB image on a cold cache.
//! `cargo test` must stay runnable on a laptop with no daemon and on a CI job
//! that did not ask for one, so these are opt-in:
//!
//! ```text
//! cargo test -p casual-task-persistence -- --ignored
//! ```
//!
//! The cost of that choice, stated: an ignored test is one nobody runs. It is
//! acceptable here only because the same invariants are gated unconditionally
//! by `scripts/verify-schema.sh` in CI — this harness is a *seam*, not a second
//! gate, and the day it becomes the only thing asserting something it must
//! stop being ignored.

use anyhow::{Context, Result};
use sqlx::{Executor, PgPool, Row};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

/// A running PostgreSQL with every migration applied.
///
/// The container is owned by the returned value: dropping it stops and removes
/// the container, so a test that panics does not leak one.
#[allow(missing_debug_implementations)]
pub struct TestDatabase {
    pub pool: PgPool,
    port: u16,
    // Held solely for its `Drop`. Never read.
    _container: testcontainers::ContainerAsync<Postgres>,
}

impl TestDatabase {
    /// Start PostgreSQL 16 and apply `migrations/` in lexical order.
    ///
    /// 16 specifically, and not `latest`: `docs/18` §Server platform pins
    /// **16+** because `UNIQUE NULLS NOT DISTINCT` is load-bearing for
    /// workspace-scoped tag uniqueness. A harness that silently drifted to a
    /// newer major would stop testing the floor the product supports.
    pub async fn start() -> Result<Self> {
        let container = Postgres::default()
            .with_tag("16-alpine")
            .start()
            .await
            .context(
                "starting PostgreSQL — this test needs a working Docker daemon; \
                 it is #[ignore] precisely so that is opt-in",
            )?;

        let port = container.get_host_port_ipv4(5432).await?;
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        let pool = PgPool::connect(&url).await.context("connecting")?;

        for (name, sql) in migrations()? {
            pool.execute(sql.as_str())
                .await
                .with_context(|| format!("applying {name}"))?;
        }

        Ok(Self {
            pool,
            port,
            _container: container,
        })
    }

    /// A DSN for the non-superuser application role.
    ///
    /// Tests that assert row-level security MUST connect as this role: RLS is
    /// inert for a superuser (migration 0012), so the same assertions run as
    /// the owner would pass while proving nothing.
    // Each integration test is its own binary and compiles this module
    // separately, so it is dead code in the ones that connect as the owner.
    #[allow(dead_code)]
    pub fn app_url(&self) -> String {
        format!(
            "postgres://taskforge_app:apppw@127.0.0.1:{}/postgres",
            self.port
        )
    }

    /// A DSN for the dispatcher role (migration 0014).
    ///
    /// Separate from [`Self::app_url`] because the two roles differ in exactly
    /// the property under test: this one bypasses row-level security, and a
    /// test that used the app role would prove the opposite of what it claims.
    // Each test binary compiles this module separately, so it is dead code in
    // the ones that do not connect as the dispatcher.
    #[allow(dead_code)]
    pub fn dispatcher_url(&self) -> String {
        format!(
            "postgres://taskforge_dispatcher:dispw@127.0.0.1:{}/postgres",
            self.port
        )
    }
}

/// Every `migrations/*.sql`, in the order the filenames impose.
///
/// Sorted rather than read in directory order: `read_dir` is unordered, and
/// applying 0008 before 0001 fails in a way that looks like a schema bug.
fn migrations() -> Result<Vec<(String, String)>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no migrations found in {}",
        dir.display()
    );

    files
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let sql = std::fs::read_to_string(&p).with_context(|| format!("reading {name}"))?;
            Ok((name, sql))
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn every_migration_applies_to_a_clean_postgres_16() -> Result<()> {
    let db = TestDatabase::start().await?;

    // A schema that applied but produced no tables would satisfy a naive
    // "did it error?" check.
    let tables: i64 = sqlx::query("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")
        .fetch_one(&db.pool)
        .await?
        .get(0);
    assert!(
        tables > 20,
        "only {tables} tables after applying every migration"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_harness_reaches_the_invariants_the_shell_gate_asserts() -> Result<()> {
    // The point of this test is not the assertions — verify-schema.sh already
    // gates them, unconditionally, in CI. It is that a Rust test can reach
    // them, which is what Phase 1's per-endpoint tenant-isolation suites need.
    let db = TestDatabase::start().await?;

    let unprotected: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname
           FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace
           JOIN pg_attribute a ON a.attrelid = c.oid
          WHERE n.nspname = 'public'
            AND c.relkind IN ('r', 'p')
            AND a.attname = 'workspace_id'
            AND NOT c.relrowsecurity
            AND c.relname <> 'outbox_event'",
    )
    .fetch_all(&db.pool)
    .await?;
    assert!(
        unprotected.is_empty(),
        "tables carry workspace_id without row-level security: {unprotected:?}"
    );

    let app_is_superuser: bool =
        sqlx::query_scalar("SELECT rolsuper FROM pg_roles WHERE rolname = 'taskforge_app'")
            .fetch_one(&db.pool)
            .await?;
    assert!(
        !app_is_superuser,
        "the application role is a superuser, which makes RLS inert (migration 0012)"
    );
    Ok(())
}
