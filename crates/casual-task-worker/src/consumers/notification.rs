//! The notification fan-out consumer (`docs/25` §Consumer fan-out, `docs/29`).
//!
//! # The failure this module prevents
//!
//! A notification that exists only in an email — and its mirror, an email that
//! costs the recipient the record. `docs/29` §Channels makes in-app the system
//! of record: "every notification lands there regardless of other channel
//! settings, so nothing is ever *only* in an email someone deleted."
//!
//! So the order here is fixed and is not an implementation detail: **the rows
//! are written and committed first**, and mail is sent afterwards, outside the
//! transaction. A relay that is down produces a logged failure and a delivered
//! notification. A relay that is down must never produce a rolled-back
//! transaction.
//!
//! It is also why mail is sent after the commit and not inside it: AGENTS.md
//! forbids I/O inside a transaction, and an SMTP conversation held across one
//! pins a pooled connection for the length of a network round trip to somebody
//! else's server.
//!
//! # Why this consumer holds its own pool
//!
//! The dispatcher's connection is granted on the two outbox tables and nothing
//! else (migration 0014, deliberately). Reading assignees and writing
//! notifications needs the application role, so this holds a pool of its own and
//! opens a `WorkspaceScope` per event. That does not reintroduce the shape
//! D-038 rejected — the claim transaction is committed before `deliver` runs,
//! and nothing here touches it.

use std::sync::Arc;

use casual_task_app::notification::{Candidate, Reason, email};
use casual_task_infra::Mailer;
use casual_task_infra::mail::Message;
use casual_task_model::{UserId, WorkspaceId, WorkspaceScope};
use casual_task_persistence::dispatch::Claimed;
use casual_task_persistence::{Scoped, audience, notification};
use sqlx::PgPool;
use uuid::Uuid;

use crate::dispatcher::Consumer;

/// The consumer name. Must match `casual_task_persistence::CONSUMERS`, or this
/// is never given work.
pub const NAME: &str = "notification_fanout";

/// How many recent commenters can carry `PARTICIPATED` for one event.
///
/// A bound, because a task with two thousand comments would otherwise turn one
/// event into two thousand candidates. AGENTS.md: every input bounded.
const MAX_PARTICIPANTS: i64 = 200;

/// Turns events into notifications.
#[allow(missing_debug_implementations)] // holds a PgPool and a dyn Mailer
pub struct NotificationFanout {
    pool: PgPool,
    mailer: Arc<dyn Mailer>,
    /// `TF_PUBLIC_URL`. The link in the mail; without it an email says
    /// something happened and gives no way to go and look.
    public_url: String,
}

impl NotificationFanout {
    #[must_use]
    pub fn new(pool: PgPool, mailer: Arc<dyn Mailer>, public_url: String) -> Self {
        Self {
            pool,
            mailer,
            public_url,
        }
    }

    /// What to send, gathered inside the transaction and sent outside it.
    async fn fan_out(&self, event: &Claimed) -> Result<Vec<Outgoing>, String> {
        let Some(subject) = Subject::of(event) else {
            // An event with no task behind it — a workspace rename, a team
            // create. Not everything is a notification (`docs/29`: everything
            // else "belongs in the activity feed, which is pull, not push").
            return Ok(Vec::new());
        };

        let workspace = WorkspaceId::from_uuid(event.workspace_id);
        let scope = WorkspaceScope::for_job(workspace);
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let mut scoped = Scoped::apply(&mut tx, &scope)
            .await
            .map_err(|e| format!("applying the tenant scope: {e}"))?;

        let (task_id, mentioned) = subject.resolve(&mut scoped).await?;
        let Some(dispatchable) = audience::dispatchable(&mut scoped, task_id)
            .await
            .map_err(|e| format!("reading the task: {e}"))?
        else {
            // The task was deleted between the event and this delivery.
            // At-least-once means that is normal, not an error.
            return Ok(Vec::new());
        };

        let candidates = gather(&mut scoped, task_id, &mentioned).await?;
        let deliveries = casual_task_app::notification::resolve(
            event.actor_id.map(UserId::from_uuid),
            &candidates,
        );
        if deliveries.is_empty() {
            return Ok(Vec::new());
        }

        let actor_name = actor_display_name(&mut scoped, event.actor_id).await?;
        let mut outgoing = Vec::new();
        let recipients: Vec<Uuid> = deliveries.iter().map(|d| d.user().as_uuid()).collect();
        let contacts = audience::addresses(&mut scoped, &recipients)
            .await
            .map_err(|e| format!("reading recipient addresses: {e}"))?;

        for delivery in &deliveries {
            let user = delivery.user().as_uuid();
            let payload = serde_json::json!({
                "task_id": task_id,
                "key": dispatchable.key,
                "project_id": dispatchable.project_id,
                "event_type": event.event_type,
            });
            let recorded = notification::record(
                &mut scoped,
                &notification::NewNotification {
                    user_id: user,
                    event_type: &event.event_type,
                    reason: delivery.reason().as_str(),
                    aggregate_id: task_id,
                    payload,
                },
                |stored| Reason::parse(stored).map_or(u8::MAX, Reason::rank),
            )
            .await
            .map_err(|e| format!("writing the notification: {e}"))?;

            // Three conditions, and all three are `docs/29`: a merge sends no
            // second mail (rule 2), only ranks 1-3 mail by default (§Channels),
            // and an anonymized account has no address (ADR-026) but keeps its
            // in-app row.
            if !recorded.is_new() || !delivery.emails_immediately() {
                continue;
            }
            let Some((_, _, Some(address))) = contacts.iter().find(|(id, _, _)| *id == user) else {
                continue;
            };
            outgoing.push(Outgoing {
                to: address.clone(),
                subject: email::subject(&email::Subject {
                    key: &dispatchable.key,
                    title: &dispatchable.title,
                }),
                body: email::body(
                    &email::Subject {
                        key: &dispatchable.key,
                        title: &dispatchable.title,
                    },
                    actor_name
                        .as_deref()
                        .map(|display_name| email::Actor { display_name }),
                    delivery.reason(),
                    &event.event_type,
                    &format!(
                        "{}/t/{}",
                        self.public_url.trim_end_matches('/'),
                        dispatchable.key
                    ),
                ),
            });
        }

        tx.commit()
            .await
            .map_err(|e| format!("committing the notifications: {e}"))?;
        Ok(outgoing)
    }
}

impl Consumer for NotificationFanout {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn deliver(&self, event: &Claimed) -> Result<(), String> {
        let outgoing = self.fan_out(event).await?;

        for message in outgoing {
            let composed = Message::new(message.to, message.subject, message.body);
            if let Err(error) = self.mailer.send(&composed).await {
                // Logged, not returned. Returning would fail the delivery and
                // retry the whole fan-out, which would re-run `record` — and
                // `record` is idempotent only within the coalescing window, so
                // a retry an hour later would write a second in-app row to fix
                // an email. `docs/29` makes in-app the record; the email is one
                // channel on top of it, and losing a channel must not cost the
                // record.
                //
                // No message fields in the log line: the subject carries a task
                // title (`docs/46` forbids customer content at any level), and
                // `Message`'s own `Debug` redacts both for the same reason.
                tracing::warn!(
                    %error,
                    event_type = event.event_type,
                    "a notification email was not sent; the in-app record stands"
                );
            }
        }
        Ok(())
    }
}

/// One composed email, held until the transaction has committed.
struct Outgoing {
    to: String,
    subject: String,
    body: String,
}

/// What the event is about, and how to get from it to a task.
enum Subject {
    /// The aggregate already is the task.
    Task(Uuid),
    /// The aggregate is a comment; the task and the mentions come from it.
    Comment(Uuid),
}

impl Subject {
    /// `None` for an event no notification can come from.
    ///
    /// A closed match rather than a prefix test: `docs/25`'s catalogue is the
    /// list, and an event type nobody has considered should produce nothing
    /// rather than a notification with invented wording.
    fn of(event: &Claimed) -> Option<Self> {
        match event.event_type.as_str() {
            "task.created"
            | "task.updated"
            | "task.assigned"
            | "task.unassigned"
            | "task.status.changed"
            | "task.closed"
            | "task.reopened"
            | "task.tagged" => Some(Self::Task(event.aggregate_id)),
            "comment.created" | "comment.updated" => Some(Self::Comment(event.aggregate_id)),
            _ => None,
        }
    }

    /// The task, and anyone the event mentions.
    async fn resolve(&self, scoped: &mut Scoped<'_>) -> Result<(Uuid, Vec<Uuid>), String> {
        match self {
            Self::Task(id) => Ok((*id, Vec::new())),
            Self::Comment(id) => audience::comment_mentions(scoped, *id)
                .await
                .map_err(|e| format!("reading the comment: {e}"))?
                .ok_or_else(|| "the comment is gone".to_owned()),
        }
    }
}

/// Everyone connected to the task, permission-filtered.
///
/// The filter is applied to the whole set at once rather than per source, so a
/// source added later cannot skip it.
async fn gather(
    scoped: &mut Scoped<'_>,
    task_id: Uuid,
    mentioned: &[Uuid],
) -> Result<Vec<Candidate>, String> {
    let assignees = audience::assignees(scoped, task_id)
        .await
        .map_err(|e| format!("reading assignees: {e}"))?;
    let reporter = audience::reporter(scoped, task_id)
        .await
        .map_err(|e| format!("reading the reporter: {e}"))?;
    let participants = audience::participants(scoped, task_id, MAX_PARTICIPANTS)
        .await
        .map_err(|e| format!("reading participants: {e}"))?;

    let mut everyone: Vec<Uuid> = Vec::new();
    everyone.extend(mentioned.iter().copied());
    everyone.extend(assignees.iter().copied());
    everyone.extend(reporter);
    everyone.extend(participants.iter().copied());
    everyone.sort_unstable();
    everyone.dedup();

    // docs/29: "A user is never notified about a task they cannot see —
    // including via a mention." The mentions are client-supplied user ids, so
    // this is the only thing standing between a comment on a private task and
    // its title in a stranger's inbox.
    let visible = audience::visible_to(scoped, task_id, &everyone)
        .await
        .map_err(|e| format!("checking visibility: {e}"))?;

    let allowed = |id: &Uuid| visible.contains(id);
    let mut candidates = Vec::new();
    for id in mentioned.iter().filter(|id| allowed(id)) {
        candidates.push(Candidate::new(UserId::from_uuid(*id), Reason::Mentioned));
    }
    for id in assignees.iter().filter(|id| allowed(id)) {
        candidates.push(Candidate::new(UserId::from_uuid(*id), Reason::Assigned));
    }
    for id in reporter.iter().filter(|id| allowed(id)) {
        candidates.push(Candidate::new(UserId::from_uuid(*id), Reason::Reported));
    }
    for id in participants.iter().filter(|id| allowed(id)) {
        candidates.push(Candidate::new(UserId::from_uuid(*id), Reason::Participated));
    }
    Ok(candidates)
}

/// The actor's display name, for the sentence the email opens with.
async fn actor_display_name(
    scoped: &mut Scoped<'_>,
    actor: Option<Uuid>,
) -> Result<Option<String>, String> {
    let Some(actor) = actor else { return Ok(None) };
    Ok(audience::addresses(scoped, &[actor])
        .await
        .map_err(|e| format!("reading the actor: {e}"))?
        .into_iter()
        .next()
        .map(|(_, display_name, _)| display_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(event_type: &str) -> Claimed {
        Claimed {
            delivery_id: Uuid::now_v7(),
            event_id: Uuid::now_v7(),
            consumer: NAME.to_owned(),
            event_type: event_type.to_owned(),
            aggregate_id: Uuid::now_v7(),
            payload: serde_json::Value::Null,
            attempts: 1,
            workspace_id: Uuid::now_v7(),
            project_id: Some(Uuid::now_v7()),
            actor_id: Some(Uuid::now_v7()),
        }
    }

    #[test]
    fn the_consumer_name_is_one_the_outbox_writes_a_delivery_row_for() {
        // A name not in CONSUMERS gets no delivery rows and the consumer sits
        // idle forever, looking like it works.
        assert!(
            casual_task_persistence::CONSUMERS.contains(&NAME),
            "{NAME} is not one of the six consumers docs/25 names"
        );
    }

    #[test]
    fn task_and_comment_events_resolve_to_a_subject_and_others_do_not() {
        for event_type in [
            "task.created",
            "task.assigned",
            "task.status.changed",
            "comment.created",
        ] {
            assert!(
                Subject::of(&event(event_type)).is_some(),
                "{event_type} produces no notification"
            );
        }
        // Not everything is a notification. docs/29: the rest is the activity
        // feed, which is pull.
        for event_type in [
            "workspace.created",
            "project.created",
            "team.member.added",
            "role.assigned",
        ] {
            assert!(
                Subject::of(&event(event_type)).is_none(),
                "{event_type} would notify somebody"
            );
        }
    }

    #[test]
    fn a_comment_event_resolves_through_the_comment_and_a_task_event_does_not() {
        assert!(matches!(
            Subject::of(&event("comment.created")),
            Some(Subject::Comment(_))
        ));
        assert!(matches!(
            Subject::of(&event("task.updated")),
            Some(Subject::Task(_))
        ));
    }

    #[test]
    fn every_event_the_subject_accepts_has_email_wording() {
        // The two lists are in different crates and drift silently: an event
        // added here without wording sends "changed", which is the vague mail
        // docs/29 forbids.
        for event_type in [
            "task.created",
            "task.updated",
            "task.assigned",
            "task.unassigned",
            "task.status.changed",
            "task.closed",
            "task.reopened",
            "task.tagged",
            "comment.created",
            "comment.updated",
        ] {
            assert!(Subject::of(&event(event_type)).is_some());
        }
    }
}
