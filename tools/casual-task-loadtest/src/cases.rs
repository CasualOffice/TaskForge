//! The case catalogue: one entry per query shape, each tied to the row of
//! `docs/30` §Server-side latency targets it contributes to.
//!
//! Every statement here is the SQL from `docs/26-SEARCH-INDEXING-AND-QUERY.md`
//! for the corresponding read path, so a plan regression shows up here at the
//! same time it would show up in the `EXPLAIN` no-seq-scan suite (F-008).
//! Parameters are `psql` variables bound by [`crate::fixtures`]; nothing here
//! interpolates a string.
//!
//! Cases are **read-only by construction**. See the crate docs for why no write
//! case is measured in Phase 0.

/// One measurable query shape.
#[derive(Debug, Clone, Copy)]
pub struct Case {
    /// Stable identifier. It is the join key between a report and a committed
    /// baseline, so renaming one is a breaking change to every baseline file.
    pub id: &'static str,
    /// The `docs/30` latency-table row this contributes to. `None` means the
    /// case measures the harness rather than the product.
    pub target: Option<&'static str>,
    /// What the case is for, in one line. Printed by `cases`.
    pub summary: &'static str,
    /// A single SQL statement, without the trailing semicolon.
    pub sql: &'static str,
}

/// The catalogue. Order is the order cases are measured and reported.
pub const CASES: &[Case] = &[
    Case {
        id: "roundtrip_floor",
        target: None,
        summary: "Protocol floor: the cost of a round trip that does no work. \
                  Read every other case as this much plus query cost.",
        sql: "SELECT 1",
    },
    Case {
        id: "task_read_by_id",
        target: Some("Task read"),
        summary: "Single task by primary key, joined to its project and status.",
        sql: "\
SELECT t.id, t.number, t.title, t.state, t.priority, t.position, t.version,
       p.key AS project_key, s.name AS status_name, s.state AS status_state
  FROM task t
  JOIN project p ON p.id = t.project_id
  JOIN workflow_status s ON s.id = t.status_id
 WHERE t.id = :'task_id'::uuid
   AND t.deleted_at IS NULL",
    },
    Case {
        id: "task_read_by_key",
        target: Some("Task read"),
        summary: "Task by human key (project, number) — the WR-125 lookup, \
                  served by task_key_uq.",
        sql: "\
SELECT t.id, t.title, t.state, t.priority
  FROM task t
 WHERE t.project_id = :'project_id'::uuid
   AND t.number = :task_number
   AND t.deleted_at IS NULL",
    },
    Case {
        id: "board_column_load",
        target: Some("Board load (200 cards)"),
        summary: "One board column at the docs/30 p95 column size, ordered by \
                  lexicographic rank (task_board_ix).",
        sql: "\
SELECT t.id, t.number, t.title, t.priority, t.position, t.due_at
  FROM task t
 WHERE t.project_id = :'project_id'::uuid
   AND t.status_id = :'status_id'::uuid
   AND t.deleted_at IS NULL
 ORDER BY t.position
 LIMIT 200",
    },
    Case {
        id: "list_page_cursor",
        target: Some("List/filter page (50)"),
        summary: "Keyset page of 50 (+1 to detect a next page) using the \
                  (updated_at, id) cursor from docs/26 §Cursor pagination.",
        sql: "\
SELECT t.id, t.number, t.title, t.updated_at
  FROM task t
 WHERE t.project_id = :'project_id'::uuid
   AND t.deleted_at IS NULL
   AND (t.updated_at, t.id) < (:'cursor_updated_at'::timestamptz, :'cursor_id'::uuid)
 ORDER BY t.updated_at DESC, t.id DESC
 LIMIT 51",
    },
    Case {
        id: "full_text_search",
        target: Some("Full-text search"),
        summary: "The docs/26 §Permission filtering query verbatim: projection \
                  table, GIN, permission pre-filter, ranked, LIMIT 51.",
        sql: "\
SELECT t.id, ts_rank_cd(s.document, q) AS rank
  FROM task_search s
  JOIN task t ON t.id = s.task_id,
       plainto_tsquery('english', :'search_term') q
 WHERE s.workspace_id = :'workspace_id'::uuid
   AND s.project_id = ANY(:'accessible_projects'::uuid[])
   AND s.document @@ q
   AND t.deleted_at IS NULL
 ORDER BY rank DESC, t.id DESC
 LIMIT 51",
    },
    Case {
        id: "my_work_assigned",
        target: Some("My Work"),
        summary: "Open work assigned to one actor across every project \
                  (task_assignee_user_ix into task).",
        sql: "\
SELECT t.id, t.number, t.title, t.due_at, t.state
  FROM task t
  JOIN task_assignee a ON a.task_id = t.id
 WHERE a.user_id = :'user_id'::uuid
   AND a.workspace_id = :'workspace_id'::uuid
   AND t.state IN ('PLANNED', 'ACTIVE')
   AND t.deleted_at IS NULL
 ORDER BY t.due_at NULLS LAST, t.id
 LIMIT 51",
    },
    Case {
        id: "permission_resolution_cold",
        target: Some("Permission resolution (cold)"),
        summary: "The role_assignment × role_permission union from docs/04, \
                  with team principal expansion. The QUERY only — the resolver \
                  itself is Phase 1.",
        sql: "\
SELECT ra.scope_type, ra.scope_id, rp.permission
  FROM role_assignment ra
  JOIN role_permission rp ON rp.role_id = ra.role_id
 WHERE ra.workspace_id = :'workspace_id'::uuid
   AND (
        (ra.principal_type = 'USER' AND ra.principal_id = :'user_id'::uuid)
     OR (ra.principal_type = 'TEAM' AND ra.principal_id IN (
            SELECT tm.team_id FROM team_membership tm
             WHERE tm.user_id = :'user_id'::uuid))
       )",
    },
    Case {
        id: "accessible_projects",
        target: Some("Permission resolution (cold)"),
        summary: "The accessible project set resolved once per list (docs/04 \
                  §The list problem). Its cardinality is the scaling limit in \
                  docs/30 §Known scaling limits.",
        sql: "\
SELECT DISTINCT p.id
  FROM project p
  JOIN role_assignment ra
    ON ra.workspace_id = p.workspace_id
   AND ((ra.scope_type = 'PROJECT' AND ra.scope_id = p.id)
     OR (ra.scope_type = 'WORKSPACE' AND ra.scope_id = p.workspace_id))
 WHERE p.workspace_id = :'workspace_id'::uuid
   AND p.deleted_at IS NULL
   AND (
        (ra.principal_type = 'USER' AND ra.principal_id = :'user_id'::uuid)
     OR (ra.principal_type = 'TEAM' AND ra.principal_id IN (
            SELECT tm.team_id FROM team_membership tm
             WHERE tm.user_id = :'user_id'::uuid))
       )",
    },
    Case {
        id: "activity_page",
        target: Some("Activity page"),
        summary: "A task's history tab: partitioned activity_event via \
                  activity_stream_ix.",
        sql: "\
SELECT e.id, e.event_type, e.actor_id, e.occurred_at, e.changes
  FROM activity_event e
 WHERE e.workspace_id = :'workspace_id'::uuid
   AND e.aggregate_id = :'task_id'::uuid
 ORDER BY e.occurred_at DESC
 LIMIT 50",
    },
];

/// Operations from the `docs/30` latency table that this harness deliberately
/// does not approximate. Emitted in every report so the covered set can never
/// be mistaken for the whole table.
#[derive(Debug, Clone, Copy)]
pub struct Gap {
    pub operation: &'static str,
    pub reason: &'static str,
    pub arrives: &'static str,
}

pub const GAPS: &[Gap] = &[
    Gap {
        operation: "Task create",
        reason: "A write measured with COMMIT mutates the corpus and makes the \
                 run non-repeatable; measured with ROLLBACK it omits the WAL \
                 flush, which is the dominant cost. Neither is worth a baseline.",
        arrives: "Phase 1 (C-004), against a disposable corpus",
    },
    Gap {
        operation: "Task update",
        reason: "As Task create, plus optimistic-concurrency retry cost that \
                 does not exist until the command layer does.",
        arrives: "Phase 1 (C-004)",
    },
    Gap {
        operation: "Status transition",
        reason: "A transition is a command with guard evaluation, dependency \
                 checks, and four writes in one transaction (docs/23). Its SQL \
                 alone would understate it by more than it measures.",
        arrives: "Phase 1 (C-006)",
    },
    Gap {
        operation: "Permission resolution (cached)",
        reason: "There is no cache. The docs/30 target is a property of the \
                 epoch-keyed cache in docs/04, which is Phase 1 code.",
        arrives: "Phase 1 (C-003)",
    },
    Gap {
        operation: "HTTP, serialization, and middleware overhead",
        reason: "No API process exists. Every number here is a database round \
                 trip and excludes everything the server does around it.",
        arrives: "Phase 1 (C-001)",
    },
    Gap {
        operation: "Concurrent workload mix",
        reason: "Cases run single-threaded, one query in flight. Contention, \
                 pool saturation, and the read/write mix in docs/30 \
                 §Throughput are not exercised.",
        arrives: "Phase 1",
    },
];

pub fn find(id: &str) -> Option<&'static Case> {
    CASES.iter().find(|c| c.id == id)
}

pub fn print_catalogue() {
    println!("Measured cases ({}):\n", CASES.len());
    for c in CASES {
        println!("  {:<28} {}", c.id, c.target.unwrap_or("(harness floor)"));
        println!("  {:<28} {}\n", "", c.summary);
    }
    println!("Declared but NOT measured in Phase 0 ({}):\n", GAPS.len());
    for g in GAPS {
        println!("  {:<28} arrives: {}", g.operation, g.arrives);
        println!("  {:<28} {}\n", "", g.reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Upper-cased identifier-shaped tokens, so keyword checks are exact.
    fn sql_tokens(sql: &str) -> Vec<String> {
        sql.to_uppercase()
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn case_ids_are_unique() {
        // Ids are the join key to a committed baseline; a duplicate would make
        // the gate compare an arbitrary one of them.
        let ids: BTreeSet<&str> = CASES.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), CASES.len());
    }

    #[test]
    fn case_ids_are_stable_identifier_shaped() {
        for c in CASES {
            assert!(
                c.id.chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
                "{} is not snake_case ascii",
                c.id
            );
        }
    }

    #[test]
    fn every_statement_is_a_single_read() {
        for c in CASES {
            assert!(
                !c.sql.contains(';'),
                "{}: a semicolon would split this into two statements and the \
                 timing would cover only one of them",
                c.id
            );
            // Token-wise, not substring-wise: `deleted_at IS NULL` contains
            // "DELETE" and is a read.
            let tokens = sql_tokens(c.sql);
            for banned in [
                "INSERT", "UPDATE", "DELETE", "TRUNCATE", "ALTER", "DROP", "COPY",
            ] {
                assert!(
                    !tokens.iter().any(|t| t == banned),
                    "{}: cases are read-only by construction ({banned})",
                    c.id
                );
            }
            assert_eq!(
                tokens.first().map(String::as_str),
                Some("SELECT"),
                "{}",
                c.id
            );
        }
    }

    #[test]
    fn no_case_uses_offset() {
        // docs/30 §Anti-patterns: OFFSET is banned in application SQL, and a
        // benchmark that used it would be measuring a shape the product cannot
        // ship.
        for c in CASES {
            assert!(!c.sql.to_uppercase().contains("OFFSET"), "{}", c.id);
        }
    }

    #[test]
    fn every_multi_row_case_is_bounded() {
        // docs/30 §Anti-patterns: every query has a LIMIT, asserted. The
        // resolver cases are exempt: their result set is bounded by the grant
        // count, and truncating it would measure the wrong thing.
        const UNBOUNDED_BY_DESIGN: &[&str] = &[
            "roundtrip_floor",
            "task_read_by_id",
            "task_read_by_key",
            "permission_resolution_cold",
            "accessible_projects",
        ];
        for c in CASES {
            if UNBOUNDED_BY_DESIGN.contains(&c.id) {
                continue;
            }
            assert!(c.sql.to_uppercase().contains("LIMIT"), "{}", c.id);
        }
    }

    #[test]
    fn gaps_cover_every_write_row_of_the_docs_30_table() {
        let covered: BTreeSet<&str> = GAPS.iter().map(|g| g.operation).collect();
        for op in [
            "Task create",
            "Task update",
            "Status transition",
            "Permission resolution (cached)",
        ] {
            assert!(covered.contains(op), "{op} is not declared as a gap");
        }
    }

    #[test]
    fn find_locates_a_known_case_and_rejects_an_unknown_one() {
        assert!(find("task_read_by_id").is_some());
        assert!(find("task_read_by_ids").is_none());
    }
}
