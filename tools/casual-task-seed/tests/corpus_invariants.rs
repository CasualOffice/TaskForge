//! Invariants the database cannot enforce, asserted against a generated corpus.
//!
//! Both self-referencing parent columns in the schema are plain nullable
//! foreign keys:
//!
//! ```sql
//! parent_comment_id uuid REFERENCES comment(id),   -- one level of threading
//! parent_id         uuid REFERENCES task(id),
//! ```
//!
//! The comment beside the first one is the whole specification, and PostgreSQL
//! enforces none of it: a reply threaded onto a reply satisfies the foreign key
//! and loads without complaint. A corpus that drifts into deep trees would
//! therefore load cleanly and quietly measure the product against a shape it
//! does not allow — the board, the thread renderer, and every latency baseline
//! taken over it would be describing a different application.
//!
//! So it is asserted here instead, over the generated COPY text, which is the
//! last point at which the corpus is still checkable.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;

/// `(id column, parent column)` offsets, matching the `CREATE TABLE` order that
/// `copy.rs` writes positionally.
const COMMENT_ID: usize = 0;
const COMMENT_PARENT: usize = 3;
const TASK_ID: usize = 0;
const TASK_PARENT: usize = 13;

fn generate(label: &str) -> PathBuf {
    let out = std::env::temp_dir().join(format!(
        "casual-task-seed-invariants-{}-{label}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&out);
    let status = Command::new(env!("CARGO_BIN_EXE_casual-task-seed"))
        .args(["--scale", "tiny", "--out"])
        .arg(&out)
        .status()
        .expect("running the seed binary");
    assert!(status.success(), "seed exited {status}");
    out
}

/// `(id, parent)` for every row of a COPY file. `\N` is COPY text's NULL.
fn parent_edges(
    dir: &std::path::Path,
    file: &str,
    id_col: usize,
    parent_col: usize,
) -> Vec<(String, Option<String>)> {
    let text = std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            assert!(
                cols.len() > parent_col,
                "{file}: row has {} columns, expected more than {parent_col} — the \
                 column layout changed and these offsets are now wrong",
                cols.len()
            );
            let parent = match cols[parent_col] {
                "\\N" => None,
                v => Some(v.to_owned()),
            };
            (cols[id_col].to_owned(), parent)
        })
        .collect()
}

/// Assert every parent reference resolves, and that no parent is itself a
/// child. Returns how many rows carried a parent, so the caller can prove the
/// corpus actually exercises threading rather than passing by having none.
fn assert_at_most_one_level(edges: &[(String, Option<String>)], what: &str) -> usize {
    let ids: HashSet<&str> = edges.iter().map(|(id, _)| id.as_str()).collect();
    let parent_of: HashMap<&str, &str> = edges
        .iter()
        .filter_map(|(id, p)| p.as_deref().map(|p| (id.as_str(), p)))
        .collect();

    for (id, parent) in edges
        .iter()
        .filter_map(|(i, p)| p.as_deref().map(|p| (i, p)))
    {
        assert!(
            ids.contains(parent),
            "{what} {id} references parent {parent}, which is not in the file — the \
             load would fail on the foreign key"
        );
        assert_ne!(parent, id.as_str(), "{what} {id} is its own parent");
        if let Some(grandparent) = parent_of.get(parent) {
            panic!(
                "{what} {id} replies to {parent}, which itself replies to {grandparent}. \
                 The schema allows one level of threading only, and nothing in the \
                 database will reject this — only this test will."
            );
        }
    }
    parent_of.len()
}

#[test]
fn comment_threads_are_at_most_one_level_deep() {
    let out = generate("comments");
    let edges = parent_edges(&out, "21_comment.copy", COMMENT_ID, COMMENT_PARENT);
    let threaded = assert_at_most_one_level(&edges, "comment");
    let _ = std::fs::remove_dir_all(&out);

    assert!(
        threaded > 0,
        "no comment in the corpus has a parent, so this test proves nothing about \
         threading — the generator stopped producing replies"
    );
}

#[test]
fn subtasks_are_at_most_one_level_deep() {
    let out = generate("tasks");
    let edges = parent_edges(&out, "17_task.copy", TASK_ID, TASK_PARENT);
    let threaded = assert_at_most_one_level(&edges, "task");
    let _ = std::fs::remove_dir_all(&out);

    assert!(
        threaded > 0,
        "no task in the corpus has a parent, so this test proves nothing about subtasks"
    );
}
