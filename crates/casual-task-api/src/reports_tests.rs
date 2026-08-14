use super::*;

#[test]
fn the_dimension_set_is_closed() {
    assert!(dimension_of("team", "r").is_ok());
    for bad in ["title", "", "t.project_id; DROP TABLE task", "cycle_time"] {
        assert!(dimension_of(bad, "r").is_err(), "{bad:?}");
    }
}

#[test]
fn the_duration_measures_are_available_and_the_unbuilt_one_is_still_named() {
    // The dangerous failure is not the 501 — it is answering a request for
    // `p50 cycle_time` with a count and letting someone quote it.
    assert!(matches!(measure_of("count", "r"), Ok(Measure::Count)));
    for built in [
        "cycle_time",
        "p50_cycle_time",
        "p90_cycle_time",
        "avg_cycle_time",
        "lead_time",
        "throughput",
        "age",
        "created_vs_completed",
        "time_in_state",
    ] {
        assert!(measure_of(built, "r").is_ok(), "{built:?}");
    }
    // A measure outside the set is still refused by name rather than
    // quietly answered with counts.
    assert!(measure_of("sum", "r").is_err());
}

#[test]
fn time_in_state_needs_a_state_and_only_it_may_have_one() {
    let time_in_state = Measure::TimeInState(Reduce::P50);

    assert_eq!(
        state_for(time_in_state, Some("ACTIVE"), "r").expect("a permanent state"),
        "ACTIVE",
    );

    // Named, not guessed: "how long in which state" has no sensible
    // default, and picking one would answer a question nobody asked.
    assert!(state_for(time_in_state, None, "r").is_err());

    // Outside `docs/23`'s five. Refused here rather than at the database,
    // where an unknown value reaches the caller as a 500 for something they
    // typed.
    assert!(state_for(time_in_state, Some("IN_REVIEW"), "r").is_err());
    assert!(state_for(time_in_state, Some("active"), "r").is_err());

    // And refused for every other measure: a `state` beside `count` is a
    // parameter the answer ignores, and a caller who believes it narrowed
    // the report has a number that means something else.
    assert!(state_for(Measure::Count, Some("ACTIVE"), "r").is_err());
    assert!(state_for(Measure::Count, None, "r").is_ok());
}
