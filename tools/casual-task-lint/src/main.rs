//! # casual-task-lint
//!
//! Architecture lints. These enforce the boundary invariants in
//! `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md` and the banned patterns in
//! `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md` §Anti-patterns.
//!
//! They exist because a rule in a document survives until the eleventh engineer,
//! and a failing build survives indefinitely
//! (`docs/10-PROJECT-GOAL-AND-STANDARDS.md` §3).
//!
//! Run: `cargo run -p casual-task-lint`

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Domain crates may not depend on one another (invariant 1). Cross-module
/// needs go through a trait satisfied by `casual-task-app`.
const DOMAIN_CRATES: &[&str] = &[
    "casual-task-identity",
    "casual-task-project",
    "casual-task-workflow",
    "casual-task-task",
    "casual-task-activity",
    "casual-task-attachment",
    "casual-task-notification",
];

struct Violation {
    file: PathBuf,
    line: usize,
    lint: &'static str,
    message: String,
}

fn main() -> anyhow::Result<()> {
    let root = workspace_root()?;
    let mut v = Vec::new();

    check_no_cross_domain_dep(&root, &mut v)?;
    check_source_placement(&root, &mut v)?;
    check_no_io_in_transaction(&root, &mut v)?;

    if v.is_empty() {
        println!("architecture lints: clean");
        return Ok(());
    }

    eprintln!("\narchitecture lint violations ({}):\n", v.len());
    for x in &v {
        eprintln!(
            "  {}:{}\n    [{}] {}\n",
            x.file.display(),
            x.line,
            x.lint,
            x.message
        );
    }
    eprintln!("See docs/19-WORKSPACE-SCAFFOLD-DESIGN.md §Boundary invariants.");
    std::process::exit(1);
}

fn workspace_root() -> anyhow::Result<PathBuf> {
    // CARGO_MANIFEST_DIR is tools/casual-task-lint; the workspace is two up.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(here
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow::anyhow!("cannot locate workspace root"))?
        .to_path_buf())
}

/// Invariant 1 — no domain crate depends on another domain crate.
fn check_no_cross_domain_dep(root: &Path, out: &mut Vec<Violation>) -> anyhow::Result<()> {
    for crate_name in DOMAIN_CRATES {
        let manifest = root.join("crates").join(crate_name).join("Cargo.toml");
        let Ok(text) = fs::read_to_string(&manifest) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            for other in DOMAIN_CRATES {
                if other != crate_name && line.trim_start().starts_with(other) {
                    out.push(Violation {
                        file: manifest.clone(),
                        line: i + 1,
                        lint: "no-cross-domain-dep",
                        message: format!(
                            "{crate_name} depends on {other}. Domain crates are siblings: \
                             declare what you need as a trait and let casual-task-app supply it."
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Whether a source line uses SQL's `OFFSET`, as opposed to merely containing
/// the letters.
///
/// The rule used to be `line.to_uppercase().contains(" OFFSET ")`, which fires
/// on `let offset = ...` — a Rust binding named `offset` is not cursor
/// pagination — and misses a line that is exactly `OFFSET $1`, which is what
/// `OFFSET` in a multi-line SQL string actually looks like. Both cases occur in
/// this repository.
///
/// Two narrower rules, each of which a plain identifier cannot trigger:
/// inside a string literal, or at the start of a line and followed by a
/// parameter or a number.
fn mentions_sql_offset(line: &str) -> bool {
    // Odd-indexed segments of a `"`-split are the insides of string literals.
    // Approximate for escaped quotes, exact for the raw SQL strings that matter.
    let in_a_string = line.split('"').skip(1).step_by(2).any(offset_takes_a_value);

    // A continuation line of a multi-line SQL string carries no quote at all.
    // Anchored at the start so a Rust `offset:` struct field cannot reach it.
    let upper = line.to_uppercase();
    let continues_sql = upper.starts_with("OFFSET") && offset_takes_a_value(line);

    in_a_string || continues_sql
}

/// Whether `OFFSET` appears applied to a value — `OFFSET $1`, `OFFSET 20` —
/// rather than merely named.
///
/// The distinction earns its keep: `assert!(!sql.contains("OFFSET"))` is a test
/// *enforcing* this rule, and flagging it would make the lint fire on the code
/// that agrees with it.
fn offset_takes_a_value(s: &str) -> bool {
    let upper = s.to_uppercase();
    upper.match_indices("OFFSET").any(|(i, _)| {
        // Preceded by a boundary, so `BYTE_OFFSET` does not count.
        let boundary = i == 0
            || !upper.as_bytes()[i - 1].is_ascii_alphanumeric() && upper.as_bytes()[i - 1] != b'_';
        let after = &upper[i + "OFFSET".len()..];
        // SQL separates the keyword from its value; Rust's `offset: UtcOffset`
        // and `offset:` in a struct literal do not. Requiring whitespace here
        // is what tells a struct field from a LIMIT clause — without it this
        // lint fires on any field named `offset`, which it did.
        let separated = after.starts_with(|c: char| c.is_whitespace());
        let tail = after.trim_start();
        boundary
            && separated
            && tail.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '$' | ':' | '?'))
    })
}

/// Every spelling of "queue with no backpressure" this lint knows about.
///
/// Deliberately more than the two the rule started with: `unbounded_channel`
/// and `channel::unbounded` cover tokio and one futures path, and miss flume,
/// async-channel, crossbeam, and `std::sync::mpsc::channel` — which is
/// unbounded by definition and does not contain the word "unbounded" anywhere.
const UNBOUNDED_QUEUE_SPELLINGS: &[&str] = &[
    "unbounded_channel",
    "channel::unbounded",
    "mpsc::unbounded",
    "flume::unbounded",
    "async_channel::unbounded",
    "crossbeam_channel::unbounded",
    "UnboundedSender",
    "UnboundedReceiver",
    // Fully qualified on purpose. `std::sync::mpsc::channel` is unbounded,
    // `tokio::sync::mpsc::channel(n)` is bounded and correct, and the shorter
    // `sync::mpsc::channel` matches both. A backstop that flags the right
    // answer is worse than one that misses a wrong one — clippy.toml resolves
    // this properly by path.
    "std::sync::mpsc::channel",
];

/// Invariants 2–4 and the banned-pattern list, checked over crate sources.
///
/// `tools/` is walked as well as `crates/`. The rules are about how this
/// system is built, and a tool that ships in this repository can violate them
/// as easily as a crate can — the earlier crates-only walk left every harness
/// in `tools/` unchecked.
fn check_source_placement(root: &Path, out: &mut Vec<Violation>) -> anyhow::Result<()> {
    //
    // This lint's own source is excluded: it necessarily contains every banned
    // pattern as *data*, and a rule table is not a use of the thing it bans.
    let sources = walk_rs(&root.join("crates"))
        .into_iter()
        .chain(walk_rs(&root.join("tools")))
        .filter(|p| !p.starts_with(root.join("tools").join("casual-task-lint")));
    for entry in sources {
        let crate_name = crate_of(root, &entry);
        let text = fs::read_to_string(&entry)?;

        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            // Doc comments and comments describe the rules; they are not code.
            if line.starts_with("//") {
                continue;
            }
            let at = |lint, message: String| Violation {
                file: entry.clone(),
                line: i + 1,
                lint,
                message,
            };

            // Invariant 2 — all SQL lives in casual-task-persistence.
            if (line.contains("sqlx::query") || line.contains("sqlx::raw_sql"))
                && crate_name != "casual-task-persistence"
            {
                out.push(at(
                    "no-sql-outside-persistence",
                    format!("SQL in {crate_name}; all SQL belongs in casual-task-persistence."),
                ));
            }

            // Invariant 3 — all HTTP lives in casual-task-api.
            if (line.contains("axum::") || line.contains("StatusCode"))
                && crate_name != "casual-task-api"
            {
                out.push(at(
                    "no-http-outside-api",
                    format!("HTTP types in {crate_name}; no domain crate names a status code."),
                ));
            }

            // Banned: offset pagination (docs/26). It scans, and it skips or
            // duplicates rows under concurrent writes.
            if mentions_sql_offset(line) {
                out.push(at(
                    "no-offset",
                    "OFFSET is banned; use cursor pagination (docs/26).".into(),
                ));
            }

            // Banned: unbounded channels — a deferred out-of-memory crash.
            //
            // This is the BACKSTOP, not the gate. Matching source text cannot
            // see through an alias, a re-export, or a call split over two
            // lines, so the real enforcement is `clippy.toml`'s
            // `disallowed-methods`, which resolves paths after name resolution
            // and fails the build. What this adds is a readable message at the
            // exact line, and coverage of spellings that arrive with a crate
            // nobody has added yet.
            if UNBOUNDED_QUEUE_SPELLINGS.iter().any(|s| line.contains(s)) {
                out.push(at(
                    "bounded-channels",
                    "Unbounded channel; every queue needs backpressure (docs/24).".into(),
                ));
            }

            // Only the API edge may mint an AuthContext (docs/32).
            if line.contains("AuthContext::authenticated")
                && crate_name != "casual-task-api"
                && crate_name != "casual-task-model"
            {
                out.push(at(
                    // Its own rule id. It was reported as `scope-required`,
                    // which is a different invariant — a violation named after
                    // the wrong rule sends the reader to the wrong document.
                    "auth-context-at-edge",
                    format!(
                        "{crate_name} mints an AuthContext. Only the authentication \
                         middleware in casual-task-api may do this."
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// External calls may not sit between opening and committing a transaction.
///
/// This is intentionally a source-shape lint. TaskForge exposes storage,
/// scanner, mail and broadcast I/O through a small closed set of fields, so a
/// transaction span containing one of those spellings is the failure. The
/// domain types still enforce where SQL lives; this guard enforces when
/// external latency may run.
fn check_no_io_in_transaction(root: &Path, out: &mut Vec<Violation>) -> anyhow::Result<()> {
    for entry in walk_rs(&root.join("crates")) {
        let text = fs::read_to_string(&entry)?;
        for (at, marker) in io_inside_transaction(&text) {
            out.push(Violation {
                file: entry.clone(),
                line: text[..at].lines().count(),
                lint: "no-io-in-transaction",
                message: format!("external I/O `{marker}` occurs before this transaction commits"),
            });
        }
    }
    Ok(())
}

fn io_inside_transaction(text: &str) -> Vec<(usize, &'static str)> {
    const STARTS: &[&str] = &["unit::begin(", ".begin()"];
    const ENDS: &[&str] = &["unit::commit(", ".commit()"];
    const EXTERNAL: &[&str] = &[
        ".storage.",
        ".store.",
        ".mailer.",
        ".scan(",
        ".broadcast.publish(",
    ];

    let mut found = Vec::new();
    for start in positions(text, STARTS) {
        let tail = &text[start..];
        let Some(end) = positions(tail, ENDS).into_iter().min() else {
            continue;
        };
        let span = &tail[..end];
        for marker in EXTERNAL {
            if let Some(at) = span.find(marker) {
                found.push((start + at, *marker));
            }
        }
    }
    found
}

fn positions(text: &str, needles: &[&str]) -> Vec<usize> {
    needles
        .iter()
        .flat_map(|needle| text.match_indices(needle).map(|(at, _)| at))
        .collect()
}

/// The crate a file belongs to, under either `crates/` or `tools/`.
///
/// `tools/` matters now that the walk covers it: without it every violation in
/// a tool reported an empty crate name, which reads as a bug in the lint rather
/// than a finding.
fn crate_of(root: &Path, file: &Path) -> String {
    ["crates", "tools"]
        .iter()
        .find_map(|dir| {
            file.strip_prefix(root.join(dir))
                .ok()
                .and_then(|p| p.components().next())
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

fn walk_rs(dir: &Path) -> Vec<PathBuf> {
    let mut out = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.insert(p);
            }
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rust_binding_named_offset_is_not_cursor_pagination() {
        // The real line from tools/casual-task-seed that the earlier
        // `contains(" OFFSET ")` rule flagged.
        assert!(!mentions_sql_offset(
            "let offset = (i128::from(span) * i as i128 / n as i128) as i64;"
        ));
        assert!(!mentions_sql_offset("offset += 1;"));
        assert!(!mentions_sql_offset("self.offset = other.offset;"));
        assert!(!mentions_sql_offset("let byte_offset: usize = 3;"));
        // A struct field named `offset`. This fired in casual-task-search and
        // is the reason the whitespace requirement exists.
        assert!(!mentions_sql_offset("    offset: UtcOffset,"));
        assert!(!mentions_sql_offset("        offset: 0,"));
        // A test that ENFORCES the rule must not be flagged by it.
        assert!(!mentions_sql_offset(
            r#"assert!(!c.sql.to_uppercase().contains("OFFSET"), "{}", c.id);"#
        ));
    }

    #[test]
    fn sql_offset_is_caught_in_every_shape_it_is_written() {
        assert!(mentions_sql_offset(
            r#"let q = "SELECT id FROM task OFFSET $1";"#
        ));
        assert!(mentions_sql_offset(
            r#""... ORDER BY id OFFSET 20 LIMIT 10""#
        ));
        // A continuation line inside a multi-line SQL string, which has no
        // quote on it at all. This is what the old rule could not see.
        assert!(mentions_sql_offset("OFFSET $1"));
        assert!(mentions_sql_offset("OFFSET 100"));
    }

    #[test]
    fn every_unbounded_queue_spelling_is_recognised() {
        for spelling in [
            "let (tx, rx) = tokio::sync::mpsc::unbounded_channel();",
            "use futures::channel::mpsc::unbounded;",
            "let (s, r) = flume::unbounded();",
            "let (s, r) = async_channel::unbounded();",
            "let (s, r) = crossbeam_channel::unbounded();",
            "let (tx, rx) = std::sync::mpsc::channel();",
            "fn take(tx: UnboundedSender<Event>) {}",
        ] {
            assert!(
                UNBOUNDED_QUEUE_SPELLINGS
                    .iter()
                    .any(|s| spelling.contains(s)),
                "not recognised as an unbounded queue: {spelling}"
            );
        }
    }

    #[test]
    fn a_bounded_queue_is_not_flagged() {
        for ok in [
            "let (tx, rx) = tokio::sync::mpsc::channel(64);",
            "let (tx, rx) = std::sync::mpsc::sync_channel(64);",
            "let (s, r) = flume::bounded(64);",
        ] {
            assert!(
                !UNBOUNDED_QUEUE_SPELLINGS.iter().any(|s| ok.contains(s)),
                "bounded queue wrongly flagged: {ok}"
            );
        }
    }

    #[test]
    fn external_io_inside_a_transaction_is_rejected() {
        let source = "let mut tx = pool.begin().await?;\nstate.storage.read(key).await?;\ntx.commit().await?;";
        let found = io_inside_transaction(source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, ".storage.");
    }

    #[test]
    fn external_io_after_commit_is_allowed() {
        let source = "let mut tx = unit::begin(pool).await?;\nunit::commit(tx).await?;\nstate.storage.read(key).await?;";
        assert!(io_inside_transaction(source).is_empty());
    }
}
