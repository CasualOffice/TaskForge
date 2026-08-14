use super::*;
use crate::report::{BaselineMeta, fixture};

fn measured(mut r: Report) -> Report {
    r.baseline = Some(BaselineMeta {
        status: BaselineStatus::Measured,
        justification: "initial".into(),
        recorded_by: "F-007".into(),
    });
    r
}

#[test]
fn an_identical_run_passes() {
    let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
    let now = fixture("ref", "reduced", &[("a", 1_000)]);
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), 0);
    assert!(outcome.blockers.is_empty());
}

#[test]
fn a_ten_percent_regression_is_allowed_and_eleven_is_not() {
    let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
    let at_ten = fixture("ref", "reduced", &[("a", 1_100)]);
    assert_eq!(compare(&base, &at_ten, DEFAULT_TOLERANCE, 0).exit_code(), 0);

    let over = fixture("ref", "reduced", &[("a", 1_101)]);
    let outcome = compare(&base, &over, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), EXIT_REGRESSION);
    let v = outcome.regressions().next().expect("one regression");
    assert_eq!(v.case, "a");
    assert!((v.delta_pct - 10.1).abs() < 1e-9, "{}", v.delta_pct);
}

#[test]
fn an_improvement_never_fails() {
    let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
    let faster = fixture("ref", "reduced", &[("a", 10)]);
    assert_eq!(compare(&base, &faster, DEFAULT_TOLERANCE, 0).exit_code(), 0);
}

#[test]
fn a_placeholder_baseline_can_never_be_passed() {
    let mut base = fixture("ref", "reduced", &[("a", 1_000)]);
    base.baseline = Some(BaselineMeta {
        status: BaselineStatus::Placeholder,
        justification: String::new(),
        recorded_by: "F-007".into(),
    });
    // Even a run that is faster than the placeholder must not pass.
    let now = fixture("ref", "reduced", &[("a", 1)]);
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
    assert!(outcome.verdicts.is_empty());
    assert!(
        outcome.blockers[0].contains("placeholder"),
        "{:?}",
        outcome.blockers
    );
}

#[test]
fn a_plain_report_is_not_a_baseline() {
    let base = fixture("ref", "reduced", &[("a", 1_000)]);
    let now = fixture("ref", "reduced", &[("a", 1_000)]);
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
}

#[test]
fn a_different_environment_is_refused_rather_than_compared() {
    let base = measured(fixture("reference-8vcpu-32gb", "reduced", &[("a", 1_000)]));
    let now = fixture("some-laptop", "reduced", &[("a", 1_000)]);
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
    assert!(outcome.blockers.iter().any(|b| b.contains("environment")));
}

#[test]
fn a_different_corpus_scale_is_refused() {
    let base = measured(fixture("ref", "reference", &[("a", 1_000)]));
    let now = fixture("ref", "reduced", &[("a", 1_000)]);
    assert_eq!(
        compare(&base, &now, DEFAULT_TOLERANCE, 0).exit_code(),
        EXIT_NOT_COMPARABLE
    );
}

#[test]
fn a_schema_version_change_is_refused() {
    let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
    let mut now = fixture("ref", "reduced", &[("a", 1_000)]);
    now.schema_version += 1;
    assert_eq!(
        compare(&base, &now, DEFAULT_TOLERANCE, 0).exit_code(),
        EXIT_NOT_COMPARABLE
    );
}

#[test]
fn fewer_iterations_than_the_baseline_is_refused() {
    let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
    let mut now = fixture("ref", "reduced", &[("a", 1_000)]);
    now.iterations = 10;
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);

    // More is fine: extra samples never weaken a p95.
    let mut more = fixture("ref", "reduced", &[("a", 1_000)]);
    more.iterations = 5_000;
    assert_eq!(compare(&base, &more, DEFAULT_TOLERANCE, 0).exit_code(), 0);
}

#[test]
fn dropping_a_baselined_case_does_not_pass_the_gate() {
    let base = measured(fixture("ref", "reduced", &[("a", 1_000), ("b", 1_000)]));
    let now = fixture("ref", "reduced", &[("a", 1_000)]);
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
    assert!(outcome.blockers.iter().any(|b| b.contains("`b`")));
}

#[test]
fn a_new_case_is_reported_but_does_not_fail() {
    let base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
    let now = fixture("ref", "reduced", &[("a", 1_000), ("new_case", 9_999)]);
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), 0);
    assert_eq!(outcome.unbaselined, vec!["new_case".to_owned()]);
}

#[test]
fn the_noise_floor_excuses_a_small_absolute_move_only_when_asked() {
    let base = measured(fixture("ref", "reduced", &[("a", 60)]));
    let now = fixture("ref", "reduced", &[("a", 80)]); // +33%, +20 µs

    // Default behaviour is exactly docs/30: proportional only.
    assert_eq!(
        compare(&base, &now, DEFAULT_TOLERANCE, 0).exit_code(),
        EXIT_REGRESSION
    );

    // 25 µs: below the 60 µs baseline, so it is a floor rather than an off
    // switch, and still large enough to absorb the +20 µs move.
    let excused = compare(&base, &now, DEFAULT_TOLERANCE, 25);
    assert_eq!(excused.exit_code(), 0);
    assert!(excused.verdicts[0].excused_by_noise_floor);
    assert!(!excused.verdicts[0].regressed);
}

#[test]
fn a_baseline_with_too_few_samples_is_not_a_gate() {
    // `stats::P95_CONFIDENCE_MIN_SAMPLES` existed only to write a note, and
    // notes are never compared — so a baseline recorded from five
    // iterations, where the p95 is simply the largest of five samples,
    // gated as firmly as one from a thousand.
    let mut base = measured(fixture("ref", "reduced", &[("a", 1_000)]));
    base.iterations = 5;
    for case in &mut base.cases {
        case.samples = 5;
    }
    let mut now = fixture("ref", "reduced", &[("a", 1_000)]);
    now.iterations = 5;

    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
    assert!(
        outcome.blockers.iter().any(|b| b.contains("5 iterations")),
        "{:?}",
        outcome.blockers
    );
}

#[test]
fn a_noise_floor_larger_than_the_smallest_baseline_is_refused() {
    // `--tolerance` was bounded and this was not, which made it a silent
    // off switch: a large enough value excuses every case, every verdict
    // reads "under noise floor", and the run exits 0 with nothing in the
    // log to say the gate had been softened.
    let base = measured(fixture("ref", "reduced", &[("a", 60), ("b", 90_000)]));
    let now = fixture("ref", "reduced", &[("a", 6_000), ("b", 900_000)]);

    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 1_000_000);
    assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
    assert!(
        outcome
            .blockers
            .iter()
            .any(|b| b.contains("noise-floor-us")),
        "{:?}",
        outcome.blockers
    );
}

#[test]
fn the_noise_floor_does_not_excuse_a_large_absolute_move() {
    let base = measured(fixture("ref", "reduced", &[("a", 100_000)]));
    let now = fixture("ref", "reduced", &[("a", 200_000)]);
    assert_eq!(
        compare(&base, &now, DEFAULT_TOLERANCE, 250).exit_code(),
        EXIT_REGRESSION
    );
}

#[test]
fn a_zero_baseline_p95_is_refused_rather_than_dividing_by_zero() {
    let base = measured(fixture("ref", "reduced", &[("a", 0)]));
    let now = fixture("ref", "reduced", &[("a", 5)]);
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert_eq!(outcome.exit_code(), EXIT_NOT_COMPARABLE);
}

#[test]
fn blockers_suppress_verdicts_so_they_cannot_be_read_as_results() {
    let base = measured(fixture("a", "reduced", &[("a", 1_000)]));
    let now = fixture("b", "reduced", &[("a", 1_000)]);
    let outcome = compare(&base, &now, DEFAULT_TOLERANCE, 0);
    assert!(outcome.verdicts.is_empty());
}
