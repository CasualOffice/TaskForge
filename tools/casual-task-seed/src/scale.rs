//! Corpus sizes.
//!
//! `Reference` is the row in `docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md`
//! §Reference capacity, verbatim: 2,000,000 tasks, 200 projects, 500 users,
//! 20,000,000 activity events. Every latency gate in that document is measured
//! against it, so these numbers are a contract and not a tuning knob.
//!
//! The two smaller scales exist because a gate nobody can run does not get run:
//! `Tiny` is for CI and for checking that the corpus still loads, `Small` is
//! for a laptop that needs an answer in under a minute.

use clap::ValueEnum;

/// Measured footprints, so a scale can be chosen without discovering its cost
/// afterwards. Apple M4, release build; the byte counts are deterministic, the
/// times are not.
///
/// | scale | rows | on disk | generate | peak RSS |
/// | --- | ---: | ---: | ---: | ---: |
/// | `tiny` | 19,964 | 5.4 MiB | 0.1 s | ~13 MiB |
/// | `small` | 976,914 | 263 MiB | 0.4 s | ~13 MiB |
/// | `reference` | 38,981,941 | **10.2 GiB** | 18 s | ~26 MiB |
///
/// Memory is flat because rows stream to disk as they are generated; **disk is
/// not**. A `reference` run needs 10.2 GiB free before PostgreSQL has seen any
/// of it, and the load then wants comparable space again for the heap and
/// indexes. Write errors surface when the files are flushed, so a full disk is
/// reported at the end of a run rather than the moment it happens, and the
/// partial corpus stays on disk until the next run replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Scale {
    /// ~1,000 tasks, ~5 MiB. Loads in seconds; used by CI to prove the corpus
    /// still satisfies every constraint.
    Tiny,
    /// ~50,000 tasks, ~263 MiB. Enough rows for an index to be chosen over a
    /// sequential scan, which `Tiny` cannot guarantee.
    Small,
    /// The gated corpus. 2,000,000 tasks — 39 M rows and **10.2 GiB** of `COPY`
    /// text. Check free space before running it.
    Reference,
}

impl Scale {
    pub fn as_str(self) -> &'static str {
        match self {
            Scale::Tiny => "tiny",
            Scale::Small => "small",
            Scale::Reference => "reference",
        }
    }
}

/// The dimensions of one corpus.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    pub scale: Scale,
    pub users: usize,
    pub teams: usize,
    pub projects: usize,
    pub tasks: usize,
    pub workflows: usize,
    pub role_assignments: usize,
    pub workspace_tags: usize,
    pub service_accounts: usize,
    pub plugins: usize,
}

impl Plan {
    pub fn for_scale(scale: Scale) -> Self {
        match scale {
            Scale::Tiny => Plan {
                scale,
                users: 25,
                teams: 3,
                projects: 8,
                tasks: 1_000,
                workflows: 2,
                role_assignments: 40,
                workspace_tags: 12,
                service_accounts: 2,
                plugins: 2,
            },
            Scale::Small => Plan {
                scale,
                users: 120,
                teams: 8,
                projects: 40,
                tasks: 50_000,
                workflows: 3,
                role_assignments: 400,
                workspace_tags: 24,
                service_accounts: 3,
                plugins: 3,
            },
            Scale::Reference => Plan {
                scale,
                users: 500,
                teams: 20,
                projects: 200,
                tasks: 2_000_000,
                workflows: 5,
                role_assignments: 5_000,
                workspace_tags: 40,
                service_accounts: 6,
                plugins: 5,
            },
        }
    }

    /// Notifications, saved views, and outbox rows are derived rather than
    /// declared: their realistic size is a ratio of the task count, and a
    /// separate constant per scale would drift out of proportion.
    pub fn notifications(&self) -> usize {
        (self.tasks / 10).max(self.users * 2)
    }

    pub fn saved_views(&self) -> usize {
        (self.users * 2).max(8)
    }

    /// The outbox is a *queue*, not a log: dispatched rows are pruned, so the
    /// table stays small no matter how large the corpus is
    /// (`docs/25-EVENTS-OUTBOX-AND-AUDIT.md` §Dispatch).
    pub fn outbox_events(&self) -> usize {
        (self.tasks / 200).clamp(20, 20_000)
    }
}
