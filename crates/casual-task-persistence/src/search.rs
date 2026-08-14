//! The search projection (C-013, `docs/26` §The search projection).
//!
//! # Recomputed, never patched
//!
//! [`refresh`] rebuilds a task's whole search document from current state. It
//! never applies a delta, and that is what makes it safe behind an at-least-once
//! outbox: `docs/25` guarantees delivery *at least* once, so a consumer that
//! applied "add the word `retry`" would double-apply on a redelivery and would
//! diverge permanently if two events arrived out of order. A recomputation is
//! idempotent by construction — deliver it twice, deliver it late, deliver a
//! stale event after a fresh one, and the row still converges to the task as it
//! is now.
//!
//! The cost, stated: a redelivery does the full document build again rather than
//! a cheap patch. That is the correct trade for a projection whose whole purpose
//! is to be reconstructible.
//!
//! # Why the document spans other tables
//!
//! `docs/26` gives the weighting, and it is the reason this is a table and not a
//! generated column on `task`: a generated column cannot see tags, assignee
//! names, or comment bodies.
//!
//! | Weight | Content |
//! | --- | --- |
//! | `A` | task key (`WR-125`), title |
//! | `B` | tag names, assignee/reporter display names, milestone |
//! | `C` | description |
//! | `D` | comment bodies |

use uuid::Uuid;

use crate::scoped::Scoped;

/// The text-search configuration the document and every query must share.
///
/// A document built with `english` and queried with `simple` silently matches
/// nothing for any stemmed word — the failure looks like "search is broken for
/// some words", which is the hardest kind to attribute. It is a constant so the
/// projection and the compiler cannot drift apart.
pub const CONFIGURATION: &str = "english";

/// How many of a task's most recent comments contribute to its document.
///
/// `docs/26` puts comment bodies at weight `D` without bounding them, and an
/// unbounded body list makes one 900-comment incident the most expensive row in
/// the table to reproject — on every single update to it. The newest are the
/// ones anybody searches for.
pub const COMMENT_WINDOW: i64 = 50;

/// Rebuild one task's search document.
///
/// Returns `false` when the task no longer qualifies — absent, soft-deleted, or
/// in another tenant — in which case the caller should [`remove`] it. Both
/// halves are needed: a soft-deleted task that stayed in the projection would
/// keep appearing in search results, which is the same bug as never deleting it.
///
/// # Errors
///
/// Any database error.
pub async fn refresh(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<bool, sqlx::Error> {
    let workspace = scoped.workspace_id().as_uuid();
    let sql = format!(
        "INSERT INTO task_search
             (task_id, workspace_id, project_id, document, title_trgm, updated_at)
         SELECT t.id,
                t.workspace_id,
                t.project_id,
                setweight(to_tsvector('{CONFIGURATION}',
                    p.key || '-' || t.number || ' ' || t.title), 'A')
             || setweight(to_tsvector('{CONFIGURATION}', coalesce(b.text, '')), 'B')
             || setweight(to_tsvector('{CONFIGURATION}', coalesce(t.description, '')), 'C')
             || setweight(to_tsvector('{CONFIGURATION}', coalesce(d.text, '')), 'D'),
                t.title,
                t.updated_at
           FROM task t
           JOIN project p ON p.id = t.project_id
           -- Weight B: everything a person would type that is not the title —
           -- tag names, the people on the task, the milestone.
           LEFT JOIN LATERAL (
               SELECT string_agg(value, ' ') AS text FROM (
                   SELECT tg.name::text AS value
                     FROM task_tag tt JOIN tag tg ON tg.id = tt.tag_id
                    WHERE tt.task_id = t.id
                   UNION ALL
                   SELECT u.display_name
                     FROM task_assignee ta JOIN user_account u ON u.id = ta.user_id
                    WHERE ta.task_id = t.id
                   UNION ALL
                   SELECT u.display_name
                     FROM user_account u WHERE u.id = t.reporter_id
                   UNION ALL
                   SELECT m.name FROM milestone m WHERE m.id = t.milestone_id
               ) parts
           ) b ON true
           -- Weight D: the newest COMMENT_WINDOW comment bodies.
           LEFT JOIN LATERAL (
               SELECT string_agg(c.body, ' ') AS text
                 FROM (SELECT body, created_at
                         FROM comment
                        WHERE task_id = t.id AND deleted_at IS NULL
                        ORDER BY created_at DESC
                        LIMIT {COMMENT_WINDOW}) c
           ) d ON true
          WHERE t.id = $1
            AND t.workspace_id = $2
            AND t.deleted_at IS NULL
         ON CONFLICT (task_id) DO UPDATE
            SET document   = EXCLUDED.document,
                title_trgm = EXCLUDED.title_trgm,
                updated_at = EXCLUDED.updated_at,
                project_id = EXCLUDED.project_id"
    );
    let written = sqlx::query(&sql)
        .bind(task_id)
        .bind(workspace)
        .execute(scoped.conn())
        .await?
        .rows_affected();
    Ok(written > 0)
}

/// Drop a task from the projection.
///
/// Idempotent: removing a task that is not there is not an error, because an
/// at-least-once delivery of `task.deleted` will do exactly that.
///
/// # Errors
///
/// Any database error.
pub async fn remove(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM task_search WHERE task_id = $1 AND workspace_id = $2")
        .bind(task_id)
        .bind(scoped.workspace_id().as_uuid())
        .execute(scoped.conn())
        .await?;
    Ok(())
}

/// Whether a task currently has a projection row. For tests and diagnostics.
///
/// # Errors
///
/// Any database error.
pub async fn is_indexed(scoped: &mut Scoped<'_>, task_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM task_search WHERE task_id = $1 AND workspace_id = $2)",
    )
    .bind(task_id)
    .bind(scoped.workspace_id().as_uuid())
    .fetch_one(scoped.conn())
    .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_projection_and_the_compiler_share_one_configuration() {
        // A document built with `english` and queried with `simple` matches
        // nothing for any stemmed word, and the failure reads as "search is
        // broken for some words" — the hardest kind to attribute. The compiler
        // formats the same constant into its tsquery.
        let compiler = include_str!("compile_predicates.rs");
        assert!(
            compiler.contains("search::CONFIGURATION"),
            "the filter compiler no longer shares the projection's text-search \
             configuration; a stemmed term would silently match nothing"
        );
    }

    #[test]
    fn every_weight_the_design_names_is_assigned() {
        // docs/26 gives four weights and each one has content. A missing
        // setweight is not a compile error and not a test failure anywhere
        // else — it just quietly stops ranking that content.
        let sql = include_str!("search.rs");
        for weight in ['A', 'B', 'C', 'D'] {
            assert!(
                sql.contains(&format!("), '{weight}')")),
                "weight {weight} is not assigned; docs/26 names all four"
            );
        }
    }
}
