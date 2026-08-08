//! Gates on the files committed to `benchmarks/`.
//!
//! These run the built binary rather than calling into the crate, because the
//! crate is a binary and its exit codes are the contract CI depends on. They
//! catch the two ways a committed baseline rots: it stops parsing, and the
//! placeholder quietly becomes passable.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is tools/casual-task-loadtest.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn benchmarks(name: &str) -> PathBuf {
    repo_root().join("benchmarks").join(name)
}

fn compare(baseline: &str, report: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_casual-task-loadtest"))
        .args(["compare", "--baseline"])
        .arg(benchmarks(baseline))
        .arg("--report")
        .arg(benchmarks(report))
        .output()
        .expect("running the harness")
}

const EXIT_REGRESSION: i32 = 1;
const EXIT_NOT_COMPARABLE: i32 = 2;

/// Compare `baseline` against a *mutated copy* of itself.
///
/// Every test below is the same shape: take a real measurement, change one
/// thing about it that ought to be disqualifying, and check the gate refuses
/// it. Starting from the baseline's own numbers means the p95s are identical by
/// construction, so nothing but the mutation can be what fails.
fn compare_mutated(
    baseline: &str,
    label: &str,
    mutate: impl FnOnce(&mut serde_json::Value),
) -> std::process::Output {
    let text = std::fs::read_to_string(benchmarks(baseline)).expect("baseline");
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("json");
    // A report is a baseline without the `baseline` block; leaving it in would
    // fail for that reason instead of the one under test.
    doc.as_object_mut().expect("object").remove("baseline");
    mutate(&mut doc);

    let path = std::env::temp_dir().join(format!("casual-task-loadtest-{label}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&doc).expect("json")).expect("write");

    let out = Command::new(env!("CARGO_BIN_EXE_casual-task-loadtest"))
        .args(["compare", "--baseline"])
        .arg(benchmarks(baseline))
        .arg("--report")
        .arg(&path)
        .output()
        .expect("running the harness");
    let _ = std::fs::remove_file(&path);
    out
}

#[test]
fn a_measured_baseline_compared_against_itself_passes() {
    // Also proves the committed file parses under `deny_unknown_fields`.
    let out = compare("smoke-local.smoke.json", "smoke-local.smoke.json");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn the_placeholder_baseline_cannot_be_passed_by_anything() {
    let out = compare(
        "reference-8vcpu-32gb.reference.json",
        "reference-8vcpu-32gb.reference.json",
    );
    assert_eq!(
        out.status.code(),
        Some(EXIT_NOT_COMPARABLE),
        "a placeholder must never gate: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("GATE DID NOT RUN"), "{stdout}");
    assert!(stdout.contains("placeholder"), "{stdout}");
}

#[test]
fn mismatched_environments_are_refused_rather_than_compared() {
    let out = compare(
        "reference-8vcpu-32gb.reference.json",
        "smoke-local.smoke.json",
    );
    assert_eq!(out.status.code(), Some(EXIT_NOT_COMPARABLE));
}

#[test]
fn a_regression_against_a_committed_baseline_exits_one() {
    // Inflate every p95 by half and confirm the gate reacts, so the exit-code
    // contract itself is covered end to end.
    let text = std::fs::read_to_string(benchmarks("smoke-local.smoke.json")).expect("baseline");
    let mut doc: serde_json::Value = serde_json::from_str(&text).expect("json");
    for case in doc["cases"].as_array_mut().expect("cases") {
        let p95 = case["p95Us"].as_u64().expect("p95");
        case["p95Us"] = serde_json::json!(p95 * 3 / 2);
    }
    doc.as_object_mut().expect("object").remove("baseline");

    let slower = std::env::temp_dir().join("casual-task-loadtest-slower-report.json");
    std::fs::write(&slower, serde_json::to_string_pretty(&doc).expect("json")).expect("write");

    let out = Command::new(env!("CARGO_BIN_EXE_casual-task-loadtest"))
        .args(["compare", "--baseline"])
        .arg(benchmarks("smoke-local.smoke.json"))
        .arg("--report")
        .arg(&slower)
        .output()
        .expect("running the harness");
    let _ = std::fs::remove_file(&slower);

    assert_eq!(
        out.status.code(),
        Some(EXIT_REGRESSION),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("REGRESSION"));
}

#[test]
fn a_shrunken_corpus_cannot_pass_by_being_labelled_the_same() {
    // `corpusScale` is free text the caller supplies. Before the counts were
    // read, a ten-task database labelled `smoke` compared cleanly against a
    // hundred-thousand-task baseline and every case looked faster.
    let out = compare_mutated("smoke-local.smoke.json", "shrunken-corpus", |doc| {
        doc["corpus"]["tasks"] = serde_json::json!(10);
        doc["corpus"]["searchDocuments"] = serde_json::json!(0);
        doc["corpus"]["activityEvents"] = serde_json::json!(0);
        for case in doc["cases"].as_array_mut().expect("cases") {
            let p95 = case["p95Us"].as_u64().expect("p95");
            case["p95Us"] = serde_json::json!(p95 / 4);
        }
    });

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(EXIT_NOT_COMPARABLE),
        "a corpus four orders of magnitude smaller must not gate: {stdout}"
    );
    assert!(stdout.contains("corpus tasks"), "{stdout}");
}

#[test]
fn a_case_that_stopped_returning_rows_cannot_pass_as_an_improvement() {
    // The fastest possible way to pass a latency gate is to return nothing.
    // `rowsReturned` was recorded for exactly this and was not being read.
    let out = compare_mutated("smoke-local.smoke.json", "zero-rows", |doc| {
        for case in doc["cases"].as_array_mut().expect("cases") {
            case["rowsReturned"] = serde_json::json!(0);
        }
    });

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(EXIT_NOT_COMPARABLE),
        "a run where every case found nothing must not pass: {stdout}"
    );
    assert!(stdout.contains("returned 0 rows"), "{stdout}");
}

#[test]
fn an_unchanged_report_still_passes() {
    // The counterweight to the two tests above: the new blockers must not make
    // a legitimate rerun fail. Comparing a measurement against itself is the
    // strictest possible version of that.
    let out = compare_mutated("smoke-local.smoke.json", "unchanged", |_| {});
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn every_committed_baseline_declares_a_status() {
    for name in [
        "smoke-local.smoke.json",
        "reference-8vcpu-32gb.reference.json",
    ] {
        let text = std::fs::read_to_string(benchmarks(name)).expect(name);
        let doc: serde_json::Value = serde_json::from_str(&text).expect(name);
        let status = doc["baseline"]["status"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no baseline.status"));
        assert!(
            matches!(status, "measured" | "placeholder"),
            "{name}: unknown status {status}"
        );
        if status == "measured" {
            assert!(
                !doc["baseline"]["justification"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .is_empty(),
                "{name}: a measured baseline must say why it is what it is \
                 (benchmarks/README.md)"
            );
        }
    }
}

#[test]
fn the_case_catalogue_lists_what_it_does_not_measure() {
    let out = Command::new(env!("CARGO_BIN_EXE_casual-task-loadtest"))
        .arg("cases")
        .output()
        .expect("running the harness");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Declared but NOT measured"), "{stdout}");
    assert!(stdout.contains("Task create"), "{stdout}");
}
