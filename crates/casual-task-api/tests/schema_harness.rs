//! A PostgreSQL container with the migrations applied.
//!
//! **This duplicates `casual-task-persistence/tests/schema_harness.rs`.** Said
//! plainly rather than left to be discovered: integration tests are per-crate
//! binaries, and there is no test-support crate to share this from. Two copies
//! can drift, and the drift would be silent — a worker test passing against a
//! schema the persistence tests no longer describe.
//!
//! Consolidating it into a shared dev crate changes the workspace dependency
//! DAG that `docs/19` fixes and `casual-task-lint` enforces, which is a design
//! decision, not a refactor. Recorded as **D-052** rather than made here. This is the third copy.
//!
//! This copy is deliberately trimmed to the container and the migration runner.
//! The schema assertions stay in the persistence crate, so they cannot run
//! twice and disagree.

use anyhow::{Context, Result};
use sqlx::{Executor, PgPool};
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
    // Unused by the acceptance gate, which connects as the owner; kept so the
    // two copies of this harness stay comparable.
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
