//! The attachment scan consumer (`docs/28` step 4).
//!
//! What is asserted here is the property the whole pipeline rests on: **only a
//! clean verdict sets `committed_at`**, and `committed_at` is what every read of
//! an attachment requires. So an unscanned file, an infected one and one whose
//! scan failed are all invisible, and only one of the three is a bug.
//!
//! A fake scanner rather than ClamAV. The verdicts are the interesting part and
//! a daemon in the test loop would mean this suite only runs where someone has
//! installed one — the `INSTREAM` wire format is tested against strings in
//! `casual_task_infra::scanner`, which is where it belongs.

mod schema_harness;

use std::sync::Arc;

use anyhow::Result;
use casual_task_infra::scanner::{ScanError, Scanner, Verdict};
use casual_task_infra::storage::{ObjectHead, ObjectStore, StorageError};
use casual_task_persistence::dispatch::Claimed;
use casual_task_persistence::test_support;
use casual_task_worker::attachment_scan::AttachmentScan;
use casual_task_worker::dispatcher::Consumer;
use uuid::Uuid;

/// A scanner with a fixed opinion.
#[derive(Debug)]
struct Fixed(Result<Verdict, ()>);

impl Scanner for Fixed {
    fn scan<'a>(
        &'a self,
        _bytes: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Verdict, ScanError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.0
                .clone()
                .map_err(|()| ScanError::Unavailable("the fake is down".to_owned()))
        })
    }
}

/// A store that answers reads and records deletions.
#[derive(Debug, Default)]
struct Bytes {
    deleted: std::sync::Mutex<Vec<String>>,
}

impl ObjectStore for Bytes {
    fn presign_put(&self, _key: &str, _ttl: std::time::Duration) -> Result<String, StorageError> {
        Ok(String::new())
    }
    fn presign_get(&self, _key: &str, _ttl: std::time::Duration) -> Result<String, StorageError> {
        Ok(String::new())
    }
    fn head<'a>(
        &'a self,
        _key: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ObjectHead, StorageError>> + Send + 'a>,
    > {
        Box::pin(async { Ok(ObjectHead { byte_size: 4 }) })
    }
    fn read_prefix<'a>(
        &'a self,
        _key: &'a str,
        _len: usize,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<u8>, StorageError>> + Send + 'a>,
    > {
        Box::pin(async { Ok(b"data".to_vec()) })
    }
    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StorageError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.deleted
                .lock()
                .expect("not poisoned")
                .push(key.to_owned());
            Ok(())
        })
    }
    fn append<'a>(
        &'a self,
        _key: &'a str,
        _chunk: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StorageError>> + Send + 'a>>
    {
        Box::pin(async { Ok(()) })
    }
}

async fn app_pool(db: &schema_harness::TestDatabase) -> Result<sqlx::PgPool> {
    test_support::enable_app_login(&db.pool).await?;
    Ok(sqlx::PgPool::connect(&db.app_url()).await?)
}

/// A workspace, a task, and one uploaded-but-unscanned attachment on it.
///
/// Seeded as the **owner**, like every other consumer suite: row-level security
/// applies to `taskforge_app`, and a fixture that had to satisfy it would be a
/// fixture testing the policies rather than the consumer. The consumer itself
/// then runs as `taskforge_app`, which is the role that has to be able to do
/// this work.
async fn seed(pool: &sqlx::PgPool) -> Result<(Uuid, Uuid)> {
    let workspace = Uuid::now_v7();
    let user = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, "acme").await?;
    test_support::insert_user(pool, user, "dev@example.test", "Dev").await?;
    test_support::add_workspace_member(pool, workspace, user).await?;
    let task = test_support::insert_task_fixture(pool, workspace, user, "Has a file").await?;

    let attachment = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO attachment
             (id, workspace_id, task_id, object_key, filename, content_type,
              byte_size, checksum, scan_status, uploaded_by, committed_at)
         VALUES ($1, $2, $3, $4, 'notes.txt', 'text/plain', 4, 'abc', 'PENDING', $5, NULL)",
    )
    .bind(attachment)
    .bind(workspace)
    .bind(task)
    .bind(format!("{workspace}/{task}/{attachment}"))
    .bind(user)
    .execute(pool)
    .await?;

    Ok((workspace, attachment))
}

fn event(workspace: Uuid, attachment: Uuid) -> Claimed {
    Claimed {
        actor_id: None,
        delivery_id: Uuid::now_v7(),
        event_id: Uuid::now_v7(),
        workspace_id: workspace,
        project_id: None,
        consumer: casual_task_worker::attachment_scan::NAME.to_owned(),
        event_type: "attachment.uploaded".to_owned(),
        aggregate_id: attachment,
        payload: serde_json::Value::Null,
        attempts: 1,
    }
}

/// Read as the owner, for the same reason the fixture is written as one: this
/// is the assertion, not the code under test, and reading it through row-level
/// security would mean a passing test could also mean "the row is hidden".
async fn state(pool: &sqlx::PgPool, id: Uuid) -> Result<(String, bool)> {
    let row: (String, Option<time::OffsetDateTime>) =
        sqlx::query_as("SELECT scan_status, committed_at FROM attachment WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;
    Ok((row.0, row.1.is_some()))
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_clean_verdict_is_what_makes_a_file_visible() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let pool = app_pool(&db).await?;
    let (workspace, attachment) = seed(&db.pool).await?;

    // Before: stored and invisible, which is the state every upload lands in.
    assert_eq!(
        state(&db.pool, attachment).await?,
        ("PENDING".to_owned(), false)
    );

    let consumer = AttachmentScan::new(
        pool.clone(),
        Arc::new(Bytes::default()),
        Some(Arc::new(Fixed(Ok(Verdict::Clean)))),
    );
    consumer
        .deliver(&event(workspace, attachment))
        .await
        .map_err(anyhow::Error::msg)?;

    assert_eq!(
        state(&db.pool, attachment).await?,
        ("CLEAN".to_owned(), true)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_infected_file_is_never_committed_and_its_object_is_removed() -> Result<()> {
    let db = schema_harness::TestDatabase::start().await?;
    let pool = app_pool(&db).await?;
    let (workspace, attachment) = seed(&db.pool).await?;

    let store = Arc::new(Bytes::default());
    let consumer = AttachmentScan::new(
        pool.clone(),
        Arc::clone(&store) as Arc<dyn ObjectStore>,
        Some(Arc::new(Fixed(Ok(Verdict::Infected("Eicar".to_owned()))))),
    );
    consumer
        .deliver(&event(workspace, attachment))
        .await
        .map_err(anyhow::Error::msg)?;

    // `committed_at` stays NULL, which is what keeps it out of every read —
    // the status alone is not what hides it.
    assert_eq!(
        state(&db.pool, attachment).await?,
        ("INFECTED".to_owned(), false)
    );
    assert_eq!(
        store.deleted.lock().expect("not poisoned").len(),
        1,
        "the bytes of an infected file must not stay on disk",
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_scan_that_could_not_run_leaves_the_file_unscanned() -> Result<()> {
    // The failure this forbids: treating "the scanner is down" as "nothing was
    // found". An unreachable daemon must leave the attachment exactly as it
    // was, and fail the delivery so the dispatcher retries it.
    let db = schema_harness::TestDatabase::start().await?;
    let pool = app_pool(&db).await?;
    let (workspace, attachment) = seed(&db.pool).await?;

    let consumer = AttachmentScan::new(
        pool.clone(),
        Arc::new(Bytes::default()),
        Some(Arc::new(Fixed(Err(())))),
    );
    let outcome = consumer.deliver(&event(workspace, attachment)).await;

    assert!(outcome.is_err(), "a failed scan must not be acknowledged");
    assert_eq!(
        state(&db.pool, attachment).await?,
        ("PENDING".to_owned(), false)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn no_scanner_configured_is_not_a_clean_verdict() -> Result<()> {
    // D-062, countersigned: a deployment with no scanner fails closed. The
    // delivery is acknowledged — nothing about it will succeed later — and the
    // attachment stays exactly as unreadable as it was.
    let db = schema_harness::TestDatabase::start().await?;
    let pool = app_pool(&db).await?;
    let (workspace, attachment) = seed(&db.pool).await?;

    let consumer = AttachmentScan::new(pool.clone(), Arc::new(Bytes::default()), None);
    consumer
        .deliver(&event(workspace, attachment))
        .await
        .map_err(anyhow::Error::msg)?;

    assert_eq!(
        state(&db.pool, attachment).await?,
        ("PENDING".to_owned(), false)
    );
    Ok(())
}

#[test]
fn the_consumer_name_is_one_the_outbox_writes_deliveries_for() {
    assert!(
        casual_task_persistence::CONSUMERS.contains(&casual_task_worker::attachment_scan::NAME),
        "a name absent from docs/25's list is a consumer that polls forever and \
         is handed nothing",
    );
}
