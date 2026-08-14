use super::*;
use crate::scale::Scale;

/// The size distribution the reference corpus will have, asserted without
/// generating it: `project_sizes` draws from its own sub-stream, so these
/// are the numbers a real `--scale reference` run produces.
///
/// Checked against `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md`
/// §Reference capacity: 2,000,000 tasks, 200 projects, p95 around 20,000
/// per project, and nothing near the 200,000 design ceiling.
#[test]
fn reference_project_sizes_match_the_capacity_table() {
    let plan = Plan::for_scale(Scale::Reference);
    let mut det = Det::stream(20_260_101, "world").substream("project-sizes", 0);
    let mut sizes = project_sizes(&mut det, &plan);

    assert_eq!(
        sizes.iter().sum::<usize>(),
        2_000_000,
        "the corpus is quoted as two million tasks and must contain exactly that"
    );
    assert_eq!(sizes.len(), 200);

    sizes.sort_unstable();
    let p95 = sizes[sizes.len() * 95 / 100];
    let max = *sizes.last().expect("200 projects");
    assert!(
        (12_000..32_000).contains(&p95),
        "p95 tasks per project is {p95}; docs/30 says 20,000"
    );
    assert!(max < 200_000, "largest project {max} exceeds the ceiling");
    assert!(
        max > 8 * sizes[sizes.len() / 2],
        "distribution is not skewed enough: max {max}, median {}",
        sizes[sizes.len() / 2]
    );
    assert!(sizes[0] > 0, "every project has tasks");
}

/// Keys are immutable and appear in commit messages (ADR-007), so a
/// collision is not a cosmetic problem.
#[test]
fn project_keys_stay_unique_and_legal() {
    let mut used = HashSet::new();
    for i in 0..500 {
        let key = unique_key(
            vocab::PROJECT_KEYS[i % vocab::PROJECT_KEYS.len()],
            &mut used,
        );
        assert!((2..=10).contains(&key.len()), "{key} violates the CHECK");
        let mut chars = key.chars();
        assert!(chars.next().is_some_and(|c| c.is_ascii_uppercase()));
        assert!(chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
    }
    assert_eq!(used.len(), 500, "every key must be distinct");
}

#[test]
fn every_workflow_has_one_initial_status_and_all_five_states() {
    for (_, statuses) in WORKFLOWS {
        let mut seen = [false; 5];
        for (_, state) in *statuses {
            seen[labels::state_index(*state)] = true;
        }
        assert!(
            seen.iter().all(|s| *s),
            "a workflow must be able to express every state, or the task \
                 generator cannot give a task the status its state requires"
        );
        assert_eq!(
            statuses.last().map(|(_, s)| *s),
            Some(TaskState::Canceled),
            "the transition builder assumes CANCELED is last"
        );
    }
}

/// Owner is a superset of every other template, and Guest grants nothing
/// that writes (docs/04 §Built-in role templates).
#[test]
fn role_templates_are_ordered_by_power() {
    for p in permission::ALL {
        let key = p.as_str();
        assert!(role_has(0, key), "Owner must hold {key}");
        for role in 1..5 {
            if role_has(role, key) {
                assert!(
                    role_has(role - 1, key),
                    "role {role} exceeds role {}",
                    role - 1
                );
            }
        }
    }
    assert!(!role_has(4, "task.update"), "Guest must not write");
    assert!(role_has(4, "task.read"));
}
