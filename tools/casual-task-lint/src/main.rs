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

/// Invariants 2–4 and the banned-pattern list, checked over crate sources.
fn check_source_placement(root: &Path, out: &mut Vec<Violation>) -> anyhow::Result<()> {
    for entry in walk_rs(&root.join("crates")) {
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
            if line.to_uppercase().contains(" OFFSET ") {
                out.push(at(
                    "no-offset",
                    "OFFSET is banned; use cursor pagination (docs/26).".into(),
                ));
            }

            // Banned: unbounded channels — a deferred out-of-memory crash.
            if line.contains("unbounded_channel") || line.contains("channel::unbounded") {
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
                    "scope-required",
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

fn crate_of(root: &Path, file: &Path) -> String {
    file.strip_prefix(root.join("crates"))
        .ok()
        .and_then(|p| p.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
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
