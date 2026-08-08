//! The gate on the one promise this tool makes.
//!
//! `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md` §Measurement compares today's
//! p95 against a number recorded weeks ago, and that comparison is only worth
//! anything if both runs measured the same rows. So determinism is not a nice
//! property of the seed — it is the property the latency gate rests on, and an
//! undetected loss of it would not fail anything. It would quietly turn every
//! baseline into a comparison between two different corpora.
//!
//! These tests run the built binary rather than calling into the crate: the
//! artifact CI and a developer both use is the binary, and the unit tests in
//! `src/` already cover the generators individually. What is not covered
//! anywhere else is whether the whole pipeline — generation order, id
//! allocation, file writing, map iteration — composes into byte-identical
//! output. That is what is asserted here.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Every generated corpus file, in load order, as `(file name, bytes)`.
///
/// The file *names* are part of the comparison, not just the contents: a run
/// that emitted a different set of tables would otherwise pass as long as the
/// files it did emit matched.
fn corpus_files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .expect("reading the corpus directory")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "copy"))
        .map(|p| {
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .expect("utf-8 file name")
                .to_owned();
            (name, std::fs::read(&p).expect("reading a corpus file"))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        !out.is_empty(),
        "no .copy files were produced in {}",
        dir.display()
    );
    out
}

/// Generate into a fresh directory. `label` keeps concurrent tests apart —
/// `cargo test` runs them in threads of one process, so the pid alone does not.
fn generate(label: &str, seed: &str) -> PathBuf {
    let out = std::env::temp_dir().join(format!(
        "casual-task-seed-test-{}-{label}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&out);

    let status = Command::new(env!("CARGO_BIN_EXE_casual-task-seed"))
        .args(["--scale", "tiny", "--seed", seed, "--out"])
        .arg(&out)
        .status()
        .expect("running the seed binary");
    assert!(status.success(), "seed exited {status}");
    out
}

fn describe(files: &[(String, Vec<u8>)]) -> Vec<(String, usize)> {
    files.iter().map(|(n, b)| (n.clone(), b.len())).collect()
}

#[test]
fn the_same_seed_produces_byte_identical_output() {
    let a = generate("a", "20260101");
    let b = generate("b", "20260101");

    let (fa, fb) = (corpus_files(&a), corpus_files(&b));
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);

    // Compare the shape first: a mismatch here has a readable failure message,
    // where a bare `assert_eq!` on megabytes of COPY text does not.
    assert_eq!(
        describe(&fa),
        describe(&fb),
        "two runs of the same seed produced different files or different sizes"
    );

    for ((name, left), (_, right)) in fa.iter().zip(fb.iter()) {
        assert!(
            left == right,
            "{name} differs between two runs of the same seed — the corpus is no \
             longer deterministic, and every committed latency baseline compares \
             two different corpora (docs/30 §Measurement)"
        );
    }
}

#[test]
fn a_different_seed_produces_different_output() {
    // Without this the test above would still pass if the generator ignored its
    // seed entirely, or emitted constants.
    let a = generate("seed-a", "20260101");
    let b = generate("seed-b", "20260102");

    let (fa, fb) = (corpus_files(&a), corpus_files(&b));
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);

    assert_eq!(
        fa.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        fb.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "the set of tables must not depend on the seed"
    );
    assert!(
        fa.iter().zip(fb.iter()).any(|((_, l), (_, r))| l != r),
        "two different seeds produced identical corpora — the seed is not reaching \
         the generators"
    );
}

#[test]
fn the_manifest_row_counts_match_the_files_that_were_written() {
    // The manifest is what a reader trusts about a corpus they did not generate,
    // and `benchmarks/README.md` treats it as the corpus's identity. A count
    // that drifts from the file beside it is worse than no count.
    let out = generate("manifest", "20260101");
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("manifest.json")).expect("manifest"),
    )
    .expect("manifest is json");

    let tables = manifest["tables"].as_array().expect("tables");
    assert!(!tables.is_empty(), "the manifest lists no tables");

    for entry in tables {
        let file = entry["file"].as_str().expect("file");
        let rows = entry["rows"].as_u64().expect("rows");
        let text = std::fs::read_to_string(out.join(file))
            .unwrap_or_else(|e| panic!("manifest names {file}, which is not readable: {e}"));
        // COPY text format is one record per line, and the generator writes no
        // trailing blank line.
        let actual = text.lines().filter(|l| !l.is_empty()).count() as u64;
        assert_eq!(
            actual, rows,
            "{file}: the manifest says {rows} rows, the file holds {actual}"
        );
    }

    let _ = std::fs::remove_dir_all(&out);
}
