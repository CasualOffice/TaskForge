//! C-011: the unit of work and the dispatch loop, against a real PostgreSQL.
//!
//! These assert the two guarantees that cannot be checked any other way.
//!
//! **ADR-006** — "domain change + activity + audit + outbox commit in one
//! transaction". A unit test can prove the code *issues* four inserts. Only a
//! database can prove a rollback takes all four back, and that is the half that
//! matters: the failure mode is a task whose history has a hole in it.
//!
//! **D-038** — claim, commit, deliver, record. The rejected design held a
//! transaction across consumer HTTP. The test below proves the claim's lock is
//! gone once its transaction commits, by taking that lock from a second
//! connection while the "HTTP call" is still notionally in flight.
//!
//! `#[ignore]` for the same reason as every other test here: Docker.

mod schema_harness;

use anyhow::Result;
use casual_task_model::{ActorType, WorkspaceId, WorkspaceScope};
use casual_task_persistence::{CONSUMERS, Change, Provenance, Scoped, UnitOfWork, dispatch};
use sqlx::Row;
use uuid::Uuid;

fn a_change(aggregate: Uuid) -> Change {
    Change {
        aggregate_type: "TASK".into(),
        aggregate_id: aggregate,
        project_id: None,
        event_type: "task.status.changed".into(),
        activity_changes: serde_json::json!({"status": {"from": "To Do", "to": "In Progress"}}),
        audit_changes: serde_json::json!({"status": {"from": "TODO", "to": "IN_PROGRESS"}}),
        payload: serde_json::json!({"task_id": aggregate}),
        schema_version: 1,
    }
}

fn nobody() -> Provenance {
    Provenance {
        actor: None,
        actor_type: ActorType::System,
        request_id: None,
        correlation_id: None,
        ip: None,
        user_agent: None,
    }
}

async fn a_workspace(pool: &sqlx::PgPool, slug: &str) -> Result<WorkspaceId> {
    let w = WorkspaceId::new();
    sqlx::query("INSERT INTO workspace (id, name, slug) VALUES ($1, $2, $3)")
        .bind(w.as_uuid())
        .bind(slug)
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(w)
}

/// How many rows each of the four writes left behind, for one aggregate.
async fn history_of(pool: &sqlx::PgPool, aggregate: Uuid) -> Result<(i64, i64, i64, i64)> {
    let row = sqlx::query(
        "SELECT (SELECT count(*) FROM activity_event WHERE aggregate_id = $1),
                (SELECT count(*) FROM audit_event    WHERE target_id    = $1),
                (SELECT count(*) FROM outbox_event   WHERE aggregate_id = $1),
                (SELECT count(*) FROM outbox_delivery d
                   JOIN outbox_event e ON e.id = d.event_id
                  WHERE e.aggregate_id = $1)",
    )
    .bind(aggregate)
    .fetch_one(pool)
    .await?;
    Ok((row.get(0), row.get(1), row.get(2), row.get(3)))
}

/// Not a test — a compile-time assertion that taking a [`dispatch::Dispatcher`]
/// from an already-verified role is **not** a round trip.
///
/// This is the whole fix for a `pg_roles` lookup that ran inside every
/// transaction, including one per delivery outcome. If the privilege check ever
/// moves back into this constructor it becomes `async`, and this function stops
/// compiling — which is the failure mode AGENTS.md asks for over a comment
/// saying "do not do this".
#[allow(dead_code)]
fn taking_a_dispatcher_from_a_verified_role_is_not_a_round_trip<'t>(
    role: &dispatch::DispatcherRole,
    conn: &'t mut sqlx::PgConnection,
) -> dispatch::Dispatcher<'t> {
    role.dispatcher(conn)
}

#[path = "outbox/part1.rs"]
mod part1;
#[path = "outbox/part2.rs"]
mod part2;
