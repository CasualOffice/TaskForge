//! The notification fan-out, against a real PostgreSQL (C-016, `docs/29`).
//!
//! Every test here is one of `docs/29`'s own acceptance gates, or one of the
//! two rules the coordinator called non-negotiable: in-app is the record, and
//! nobody is ever told about their own action.
//!
//! The consumer is driven through `Consumer::deliver`, which is exactly how the
//! dispatch loop calls it — the loop's own contract (claim, commit, deliver,
//! record) is C-011's and is tested there.

mod schema_harness;

use std::sync::Arc;

use anyhow::Result;
use casual_task_infra::mail::{LoggingMailer, Message};
use casual_task_persistence::dispatch::Claimed;
use casual_task_persistence::test_support::{self, TaskFixture};
use casual_task_worker::dispatcher::Consumer;
use casual_task_worker::notify::{NAME, NotificationFanout};
use sqlx::PgPool;
use uuid::Uuid;

/// A mailer that records what it was asked to send, so a test can assert on the
/// channel without a relay.
#[derive(Debug, Default)]
struct Recording {
    sent: std::sync::Mutex<Vec<(String, String)>>,
}

impl Recording {
    fn sent(&self) -> Vec<(String, String)> {
        self.sent.lock().expect("not poisoned").clone()
    }
}

impl casual_task_infra::Mailer for Recording {
    fn send<'a>(
        &'a self,
        message: &'a Message,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), casual_task_infra::mail::MailError>>
                + Send
                + 'a,
        >,
    > {
        let entry = (message.to().to_owned(), message.subject().to_owned());
        Box::pin(async move {
            self.sent.lock().expect("not poisoned").push(entry);
            Ok(())
        })
    }
}

/// A mailer that always fails, for the "in-app is the record" test.
#[derive(Debug)]
struct BrokenRelay;

impl casual_task_infra::Mailer for BrokenRelay {
    fn send<'a>(
        &'a self,
        _message: &'a Message,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), casual_task_infra::mail::MailError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            Err(casual_task_infra::mail::MailError::Transport(
                "the relay is down".to_owned(),
            ))
        })
    }
}

async fn person(pool: &PgPool, email: &str) -> Result<Uuid> {
    let id = Uuid::now_v7();
    test_support::insert_user_with_password(pool, id, email, "not-a-real-hash").await?;
    Ok(id)
}

/// A workspace whose members are all workspace members, with one WORKSPACE
/// project and one task reported by `reporter`.
async fn fixture(pool: &PgPool, reporter: Uuid, members: &[Uuid]) -> Result<TaskFixture> {
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(pool, workspace, "acme").await?;
    test_support::add_workspace_member(pool, workspace, reporter).await?;
    for member in members {
        test_support::add_workspace_member(pool, workspace, *member).await?;
    }
    Ok(test_support::seed_task(pool, workspace, reporter, "WORKSPACE", "Ship the thing").await?)
}

fn event(fixture: &TaskFixture, event_type: &str, actor: Option<Uuid>, aggregate: Uuid) -> Claimed {
    Claimed {
        delivery_id: Uuid::now_v7(),
        event_id: Uuid::now_v7(),
        consumer: NAME.to_owned(),
        event_type: event_type.to_owned(),
        aggregate_id: aggregate,
        payload: serde_json::Value::Null,
        attempts: 1,
        workspace_id: fixture.workspace_id,
        actor_id: actor,
    }
}

fn fanout(pool: &PgPool, mailer: Arc<dyn casual_task_infra::Mailer>) -> NotificationFanout {
    NotificationFanout::new(
        pool.clone(),
        mailer,
        "https://tasks.example.test".to_owned(),
    )
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_assignee_is_notified_and_the_actor_is_not() -> Result<()> {
    // The non-negotiable. docs/29 rule 1: "You are never notified about your own
    // action." It is the single most common complaint about every tracker, and
    // the reason `actor_id` was added to outbox_event in migration 0022 — before
    // that it was not expressible at all.
    let db = schema_harness::TestDatabase::start().await?;
    let actor = person(&db.pool, "actor@example.com").await?;
    let assignee = person(&db.pool, "assignee@example.com").await?;
    let task = fixture(&db.pool, actor, &[assignee]).await?;
    test_support::assign_task(&db.pool, task.workspace_id, task.task_id, assignee).await?;
    // The actor is an assignee too — the case where suppression has to beat a
    // reason that would otherwise apply.
    test_support::assign_task(&db.pool, task.workspace_id, task.task_id, actor).await?;

    let mailer = Arc::new(Recording::default());
    fanout(&db.pool, mailer.clone())
        .deliver(&event(&task, "task.assigned", Some(actor), task.task_id))
        .await
        .map_err(anyhow::Error::msg)?;

    let theirs = test_support::notifications_for(&db.pool, assignee).await?;
    assert_eq!(theirs.len(), 1, "the assignee was not told: {theirs:?}");
    assert_eq!(theirs[0].0, "ASSIGNED");

    let own = test_support::notifications_for(&db.pool, actor).await?;
    assert!(
        own.is_empty(),
        "the actor was notified about their own action: {own:?}"
    );

    // And the reporter is the actor here, so REPORTED is suppressed too.
    assert_eq!(mailer.sent().len(), 1, "one email, to the assignee");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn four_applicable_reasons_produce_one_notification_at_the_highest() -> Result<()> {
    // docs/29 §Acceptance gates, the dedup test. The recipient reported the
    // task, is assigned to it, has commented on it, and is mentioned in the new
    // comment — four reasons, one notification, labelled MENTIONED.
    let db = schema_harness::TestDatabase::start().await?;
    let recipient = person(&db.pool, "everything@example.com").await?;
    let actor = person(&db.pool, "actor@example.com").await?;
    let task = fixture(&db.pool, recipient, &[actor]).await?;
    test_support::assign_task(&db.pool, task.workspace_id, task.task_id, recipient).await?;
    test_support::seed_comment(&db.pool, task.workspace_id, task.task_id, recipient, &[]).await?;
    let comment = test_support::seed_comment(
        &db.pool,
        task.workspace_id,
        task.task_id,
        actor,
        &[recipient],
    )
    .await?;

    fanout(&db.pool, Arc::new(LoggingMailer))
        .deliver(&event(&task, "comment.created", Some(actor), comment))
        .await
        .map_err(anyhow::Error::msg)?;

    let theirs = test_support::notifications_for(&db.pool, recipient).await?;
    assert_eq!(theirs.len(), 1, "{theirs:?}");
    assert_eq!(
        theirs[0].0, "MENTIONED",
        "the highest applicable reason did not win"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn mentioning_someone_who_cannot_see_the_task_notifies_nobody() -> Result<()> {
    // docs/29 §Delivery: "A user is never notified about a task they cannot see
    // — including via a mention. Mentioning someone in a private project does
    // not silently leak the task title into their inbox."
    //
    // The mention list is client-supplied user ids (migration 0006 resolves them
    // at write time), so without the visibility check anyone could mail any
    // colleague the title of any private task.
    let db = schema_harness::TestDatabase::start().await?;
    let author = person(&db.pool, "author@example.com").await?;
    let outsider = person(&db.pool, "outsider@example.com").await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    test_support::add_workspace_member(&db.pool, workspace, author).await?;
    test_support::add_workspace_member(&db.pool, workspace, outsider).await?;
    let task =
        test_support::seed_task(&db.pool, workspace, author, "PRIVATE", "Secret plan").await?;
    // The author can see it; the outsider is a workspace member and nothing more.
    test_support::add_project_member(&db.pool, workspace, task.project_id, author).await?;
    let comment =
        test_support::seed_comment(&db.pool, workspace, task.task_id, author, &[outsider]).await?;

    let mailer = Arc::new(Recording::default());
    fanout(&db.pool, mailer.clone())
        .deliver(&event(&task, "comment.created", Some(author), comment))
        .await
        .map_err(anyhow::Error::msg)?;

    let leaked = test_support::notifications_for(&db.pool, outsider).await?;
    assert!(
        leaked.is_empty(),
        "a private task leaked into a stranger's inbox: {leaked:?}"
    );
    assert!(
        mailer.sent().is_empty(),
        "a private task title was mailed to somebody who cannot see it: {:?}",
        mailer.sent()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_dead_relay_does_not_cost_the_recipient_the_notification() -> Result<()> {
    // The other non-negotiable. docs/29 §Channels: in-app is the system of
    // record, "so nothing is ever *only* in an email someone deleted". An SMTP
    // failure is logged and the delivery still succeeds — returning an error
    // would retry the whole fan-out and write a second row to fix an email.
    let db = schema_harness::TestDatabase::start().await?;
    let actor = person(&db.pool, "actor@example.com").await?;
    let assignee = person(&db.pool, "assignee@example.com").await?;
    let task = fixture(&db.pool, actor, &[assignee]).await?;
    test_support::assign_task(&db.pool, task.workspace_id, task.task_id, assignee).await?;

    let outcome = fanout(&db.pool, Arc::new(BrokenRelay))
        .deliver(&event(&task, "task.assigned", Some(actor), task.task_id))
        .await;

    assert!(
        outcome.is_ok(),
        "a dead relay failed the delivery, which would retry the fan-out: {outcome:?}"
    );
    let theirs = test_support::notifications_for(&db.pool, assignee).await?;
    assert_eq!(theirs.len(), 1, "the in-app record was lost with the email");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn changes_inside_the_window_collapse_into_one_notification() -> Result<()> {
    // docs/29 rule 2 and its acceptance gate: "10 changes in 2 minutes produce
    // one notification." Someone editing a task for two minutes must not
    // generate eight emails.
    let db = schema_harness::TestDatabase::start().await?;
    let actor = person(&db.pool, "actor@example.com").await?;
    let assignee = person(&db.pool, "assignee@example.com").await?;
    let task = fixture(&db.pool, actor, &[assignee]).await?;
    test_support::assign_task(&db.pool, task.workspace_id, task.task_id, assignee).await?;

    let mailer = Arc::new(Recording::default());
    let consumer = fanout(&db.pool, mailer.clone());
    for _ in 0..10 {
        consumer
            .deliver(&event(&task, "task.updated", Some(actor), task.task_id))
            .await
            .map_err(anyhow::Error::msg)?;
    }

    let theirs = test_support::notifications_for(&db.pool, assignee).await?;
    assert_eq!(
        theirs.len(),
        1,
        "ten changes produced {} rows",
        theirs.len()
    );
    assert_eq!(
        mailer.sent().len(),
        1,
        "a coalesced change sent a second email"
    );

    // Past the window, the next change is news again.
    test_support::age_notifications(&db.pool, "10 minutes").await?;
    consumer
        .deliver(&event(&task, "task.updated", Some(actor), task.task_id))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        test_support::notifications_for(&db.pool, assignee)
            .await?
            .len(),
        2,
        "the coalescing window never expires"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn only_ranks_one_to_three_send_mail_and_every_reason_lands_in_app() -> Result<()> {
    // docs/29 §Channels: email is "on for rank 1–3", in-app is "always on".
    // PARTICIPATED is rank 5, so the participant gets the record and no mail.
    let db = schema_harness::TestDatabase::start().await?;
    let actor = person(&db.pool, "actor@example.com").await?;
    let participant = person(&db.pool, "participant@example.com").await?;
    let task = fixture(&db.pool, actor, &[participant]).await?;
    test_support::seed_comment(&db.pool, task.workspace_id, task.task_id, participant, &[]).await?;

    let mailer = Arc::new(Recording::default());
    fanout(&db.pool, mailer.clone())
        .deliver(&event(&task, "task.updated", Some(actor), task.task_id))
        .await
        .map_err(anyhow::Error::msg)?;

    let theirs = test_support::notifications_for(&db.pool, participant).await?;
    assert_eq!(theirs.len(), 1, "the participant got no in-app record");
    assert_eq!(theirs[0].0, "PARTICIPATED");
    assert!(
        mailer.sent().is_empty(),
        "a rank-5 reason sent mail: {:?}",
        mailer.sent()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn the_email_subject_is_the_documented_threadable_shape() -> Result<()> {
    // docs/29 §Email content: `[WR-125] Task title`, "stable, so mail clients
    // thread correctly".
    let db = schema_harness::TestDatabase::start().await?;
    let actor = person(&db.pool, "actor@example.com").await?;
    let assignee = person(&db.pool, "assignee@example.com").await?;
    let task = fixture(&db.pool, actor, &[assignee]).await?;
    test_support::assign_task(&db.pool, task.workspace_id, task.task_id, assignee).await?;

    let mailer = Arc::new(Recording::default());
    fanout(&db.pool, mailer.clone())
        .deliver(&event(&task, "task.assigned", Some(actor), task.task_id))
        .await
        .map_err(anyhow::Error::msg)?;

    let sent = mailer.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "assignee@example.com");
    assert_eq!(sent[0].1, "[WR-1] Ship the thing");
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn an_event_with_no_task_behind_it_notifies_nobody() -> Result<()> {
    // docs/29: "A notification must be something the recipient would act on.
    // Everything else belongs in the activity feed, which is pull, not push."
    // A workspace rename is not a notification.
    let db = schema_harness::TestDatabase::start().await?;
    let actor = person(&db.pool, "actor@example.com").await?;
    let member = person(&db.pool, "member@example.com").await?;
    let task = fixture(&db.pool, actor, &[member]).await?;
    test_support::assign_task(&db.pool, task.workspace_id, task.task_id, member).await?;

    let mailer = Arc::new(Recording::default());
    let consumer = fanout(&db.pool, mailer.clone());
    for event_type in ["workspace.renamed", "project.created", "team.member.added"] {
        consumer
            .deliver(&event(&task, event_type, Some(actor), task.workspace_id))
            .await
            .map_err(anyhow::Error::msg)?;
    }

    assert!(
        test_support::notifications_for(&db.pool, member)
            .await?
            .is_empty()
    );
    assert!(mailer.sent().is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_system_event_notifies_everyone_connected() -> Result<()> {
    // `actor_id` is NULL for a system-generated change (migration 0022).
    // Suppressing nobody is what that means; suppressing everybody would make
    // automation silent.
    let db = schema_harness::TestDatabase::start().await?;
    let reporter = person(&db.pool, "reporter@example.com").await?;
    let assignee = person(&db.pool, "assignee@example.com").await?;
    let task = fixture(&db.pool, reporter, &[assignee]).await?;
    test_support::assign_task(&db.pool, task.workspace_id, task.task_id, assignee).await?;

    fanout(&db.pool, Arc::new(LoggingMailer))
        .deliver(&event(&task, "task.updated", None, task.task_id))
        .await
        .map_err(anyhow::Error::msg)?;

    assert_eq!(
        test_support::notifications_for(&db.pool, assignee)
            .await?
            .len(),
        1
    );
    assert_eq!(
        test_support::notifications_for(&db.pool, reporter)
            .await?
            .len(),
        1,
        "the reporter was suppressed by a NULL actor"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs Docker; run with --ignored"]
async fn a_member_removed_from_the_workspace_stops_being_notified() -> Result<()> {
    // Visibility is checked per delivery, not at assignment time. Somebody
    // removed from the workspace keeps their `task_assignee` row until an admin
    // cleans it up, and must not keep receiving mail about work they can no
    // longer open.
    let db = schema_harness::TestDatabase::start().await?;
    let actor = person(&db.pool, "actor@example.com").await?;
    let leaver = person(&db.pool, "leaver@example.com").await?;
    let workspace = Uuid::now_v7();
    test_support::insert_workspace(&db.pool, workspace, "acme").await?;
    test_support::add_workspace_member(&db.pool, workspace, actor).await?;
    let task = test_support::seed_task(&db.pool, workspace, actor, "WORKSPACE", "Ship it").await?;
    // Assigned, but never a member of the workspace.
    test_support::assign_task(&db.pool, workspace, task.task_id, leaver).await?;

    fanout(&db.pool, Arc::new(LoggingMailer))
        .deliver(&event(&task, "task.assigned", Some(actor), task.task_id))
        .await
        .map_err(anyhow::Error::msg)?;

    assert!(
        test_support::notifications_for(&db.pool, leaver)
            .await?
            .is_empty(),
        "a non-member was notified"
    );
    Ok(())
}
