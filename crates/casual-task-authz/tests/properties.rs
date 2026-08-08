//! The two property tests `docs/04` §Acceptance gates requires.
//!
//! * **Additivity** — "for random grant sets: adding a grant never removes a
//!   permission. This is the invariant the whole model rests on."
//! * **Isolation** — "no grant in workspace A ever affects a decision in
//!   workspace B."
//!
//! # Why these are generated rather than enumerated
//!
//! Both are statements about *every* grant set, and the unit tests beside the
//! resolver check the shapes somebody thought of. Additivity in particular is
//! the kind of property a plausible refactor breaks — the moment anyone adds a
//! "most specific wins" rule between grants, or a deny, or an ordering
//! dependence, additivity fails on some set nobody wrote down.
//!
//! # Why the randomness is seeded
//!
//! A failing property test that cannot be reproduced is a rumour. `SmallRng`
//! with a fixed seed makes every run identical, and widening coverage means
//! adding seeds — which appear in the failure message, so a failure names the
//! case that produced it.

use casual_task_authz::{
    Actor, Constraint, Grant, Principal, ResourceFacts, ResourceScopes, Scope, allows,
};
use casual_task_model::{
    EnvironmentId, Permission, ProjectId, TeamId, UserId, WorkspaceId, permission as perm,
};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

/// A small permission alphabet. Four is enough for the properties and keeps a
/// failure readable; the full registry adds combinations, not coverage.
const ALPHABET: &[Permission] = &[
    perm::TASK_READ,
    perm::TASK_UPDATE,
    perm::TASK_DELETE,
    perm::TASK_COMMENT,
];

/// A fixed world, so grants can be generated that actually reach a resource.
struct World {
    workspace: WorkspaceId,
    other_workspace: WorkspaceId,
    team: TeamId,
    project: ProjectId,
    environment: EnvironmentId,
    actor: UserId,
}

impl World {
    fn new() -> Self {
        Self {
            workspace: WorkspaceId::new(),
            other_workspace: WorkspaceId::new(),
            team: TeamId::new(),
            project: ProjectId::new(),
            environment: EnvironmentId::new(),
            actor: UserId::new(),
        }
    }

    fn resource(&self) -> ResourceScopes {
        ResourceScopes::project(self.workspace, self.project)
            .in_team(self.team)
            .in_environment(self.environment)
    }

    fn actor(&self) -> Actor {
        Actor::user(self.actor).in_teams(vec![self.team])
    }

    /// A grant that may or may not reach the resource: sometimes the wrong
    /// scope, sometimes another principal, sometimes constrained. Generating
    /// only *applicable* grants would make additivity trivially true.
    fn grant(&self, rng: &mut SmallRng) -> Grant {
        let scope = match rng.random_range(0..6) {
            0 => Scope::Workspace(self.workspace),
            1 => Scope::Team(self.team),
            2 => Scope::Project(self.project),
            3 => Scope::Environment(self.environment),
            // Deliberately unreachable scopes.
            4 => Scope::Project(ProjectId::new()),
            _ => Scope::Team(TeamId::new()),
        };
        let principal = match rng.random_range(0..3) {
            0 => Principal::User(self.actor),
            1 => Principal::Team(self.team),
            _ => Principal::User(UserId::new()),
        };
        let mut permissions = Vec::new();
        for p in ALPHABET {
            if rng.random_bool(0.5) {
                permissions.push(*p);
            }
        }
        let constraints = match rng.random_range(0..4) {
            0 => vec![Constraint::AssigneeIsActor],
            1 => vec![Constraint::NotExternal],
            2 => vec![Constraint::EnvironmentIn(vec![self.environment])],
            _ => Vec::new(),
        };
        Grant {
            workspace_id: self.workspace,
            principal,
            scope,
            permissions,
            constraints,
        }
    }

    fn facts(&self, rng: &mut SmallRng) -> ResourceFacts {
        ResourceFacts {
            assignees: if rng.random_bool(0.5) {
                vec![self.actor]
            } else {
                Vec::new()
            },
            reporter: None,
            actor_is_project_member: rng.random_bool(0.5),
            environment: Some(self.environment),
            actor_is_guest: rng.random_bool(0.3),
        }
    }
}

/// Every permission in the alphabet the actor currently holds.
fn granted(actor: &Actor, r: &ResourceScopes, f: &ResourceFacts, g: &[Grant]) -> Vec<Permission> {
    ALPHABET
        .iter()
        .filter(|p| allows(actor, **p, r, f, g).is_allowed())
        .copied()
        .collect()
}

#[test]
fn adding_a_grant_never_removes_a_permission() {
    // docs/04: "This is the invariant the whole model rests on." A deny rule, a
    // most-specific-wins rule between grants, or any ordering dependence would
    // break it here.
    for seed in 0..64u64 {
        let mut rng = SmallRng::seed_from_u64(seed);
        let world = World::new();
        let (actor, resource) = (world.actor(), world.resource());
        let facts = world.facts(&mut rng);

        let mut grants: Vec<Grant> = Vec::new();
        for step in 0..8 {
            let before = granted(&actor, &resource, &facts, &grants);
            grants.push(world.grant(&mut rng));
            let after = granted(&actor, &resource, &facts, &grants);

            for p in &before {
                assert!(
                    after.contains(p),
                    "seed {seed}, step {step}: adding a grant removed `{}`. \
                     Additivity is the invariant docs/04 rests on — a deny rule \
                     or a precedence rule between grants would look exactly like this.",
                    p.as_str()
                );
            }
        }
    }
}

#[test]
fn a_grant_in_another_workspace_never_changes_a_decision() {
    // docs/04: "no grant in workspace A ever affects a decision in workspace B."
    for seed in 0..64u64 {
        let mut rng = SmallRng::seed_from_u64(seed);
        let world = World::new();
        let (actor, resource) = (world.actor(), world.resource());
        let facts = world.facts(&mut rng);

        let mut grants: Vec<Grant> = (0..4).map(|_| world.grant(&mut rng)).collect();
        let baseline = granted(&actor, &resource, &facts, &grants);

        // Re-home some grants into the other workspace, changing nothing else —
        // same principal, same scope ids, same permissions. Only the tenant
        // moves, so any difference in the decision is a tenancy leak.
        for _ in 0..4 {
            let mut foreign = world.grant(&mut rng);
            foreign.workspace_id = world.other_workspace;
            foreign.permissions = ALPHABET.to_vec();
            foreign.constraints = Vec::new();
            grants.push(foreign);
        }

        let after = granted(&actor, &resource, &facts, &grants);
        assert_eq!(
            baseline, after,
            "seed {seed}: grants in another workspace changed the decision. \
             They carried every permission unconstrained, so a leak shows up \
             as permissions appearing that the tenant's own grants never gave."
        );
    }
}

#[test]
fn the_generator_produces_both_outcomes() {
    // Without this, both properties above could pass on a generator that only
    // ever produced grants reaching nothing — vacuously true, and worthless.
    let mut rng = SmallRng::seed_from_u64(7);
    let world = World::new();
    let (actor, resource) = (world.actor(), world.resource());
    let facts = ResourceFacts {
        assignees: vec![world.actor],
        environment: Some(world.environment),
        ..Default::default()
    };

    let mut any_allowed = false;
    let mut any_denied = false;
    for _ in 0..200 {
        let g = [world.grant(&mut rng)];
        for p in ALPHABET {
            if allows(&actor, *p, &resource, &facts, &g).is_allowed() {
                any_allowed = true;
            } else {
                any_denied = true;
            }
        }
    }
    assert!(any_allowed, "the generator never produced an allow");
    assert!(any_denied, "the generator never produced a deny");
}
