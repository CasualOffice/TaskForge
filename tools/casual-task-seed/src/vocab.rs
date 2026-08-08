//! Word lists.
//!
//! Original generic engineering vocabulary. Nothing here is copied from another
//! tracker's fixtures, sample data, or documentation
//! (`docs/09-REPOSITORY-AND-CONTRIBUTION.md` §clean-room).
//!
//! The lists are deliberately small. Full-text and trigram index behaviour
//! depends on lexeme *cardinality*: a vocabulary of ten thousand unique words
//! would give every posting list a length of one and make the GIN index look
//! far better than it does against real content, where a handful of terms
//! appear in a large fraction of the corpus.

pub const FIRST_NAMES: &[&str] = &[
    "Ana", "Ben", "Chidi", "Dara", "Eli", "Farah", "Gita", "Hugo", "Ines", "Jonas", "Kai", "Lena",
    "Mateo", "Nadia", "Omar", "Priya", "Quinn", "Rosa", "Sami", "Tara", "Uma", "Viktor", "Wren",
    "Xiu", "Yosef", "Zara", "Aurel", "Bianca", "Cyrus", "Dilek", "Esme", "Frida",
];

pub const LAST_NAMES: &[&str] = &[
    "Adeyemi",
    "Bergstrom",
    "Castellanos",
    "Duarte",
    "Eriksen",
    "Fontaine",
    "Gallagher",
    "Halvorsen",
    "Ibarra",
    "Jankowski",
    "Kovacs",
    "Lindqvist",
    "Moreau",
    "Nakamura",
    "Okonkwo",
    "Pavlenko",
    "Quiroga",
    "Rasmussen",
    "Silveira",
    "Tanaka",
    "Ustinov",
    "Villanueva",
    "Whitfield",
    "Ximenes",
    "Yilmaz",
    "Zeledon",
];

pub const TEAM_NAMES: &[&str] = &[
    "Platform",
    "Growth",
    "Billing",
    "Identity",
    "Mobile",
    "Data",
    "Infrastructure",
    "Design",
    "Support",
    "Search",
    "Payments",
    "Reliability",
    "Docs",
    "Integrations",
    "Security",
    "Analytics",
    "Onboarding",
    "Notifications",
    "Workflow",
    "Reporting",
];

/// Project keys must match `^[A-Z][A-Z0-9]{1,9}$` (migration 0004). Every
/// entry here is 3–6 characters, leaving room for the numeric suffix the
/// generator appends when a key repeats.
pub const PROJECT_KEYS: &[&str] = &[
    "CORE", "APEX", "ORBIT", "NOVA", "FLUX", "PRISM", "ATLAS", "VERTEX", "QUARRY", "LEDGER",
    "SIGNAL", "BEACON", "HARBOR", "FORGE", "CIPHER", "LATTICE", "SUMMIT", "DELTA", "PULSE",
    "ANCHOR", "COMPASS", "DRIFT", "EMBER", "GLACIER", "HALO", "IRIS", "JUNCTION", "KEYSTONE",
    "LUMEN", "MERIDIAN",
];

pub const PROJECT_SUFFIXES: &[&str] = &[
    "Platform",
    "Service",
    "Migration",
    "Rewrite",
    "Rollout",
    "Modernization",
    "Hardening",
    "Programme",
];

pub const COMPONENTS: &[&str] = &[
    "outbox dispatcher",
    "permission resolver",
    "board renderer",
    "search projection",
    "attachment pipeline",
    "notification fanout",
    "audit stream",
    "workflow engine",
    "import job",
    "webhook sender",
    "session store",
    "rate limiter",
    "activity feed",
    "connection pool",
    "migration runner",
    "plugin sandbox",
];

pub const VERBS: &[&str] = &[
    "Fix",
    "Investigate",
    "Refactor",
    "Document",
    "Harden",
    "Optimise",
    "Migrate",
    "Remove",
    "Add",
    "Rework",
    "Instrument",
    "Backfill",
    "Deprecate",
    "Split",
    "Cache",
    "Throttle",
];

pub const NOUNS: &[&str] = &[
    "retry loop",
    "cursor encoding",
    "index selection",
    "cache key",
    "batch size",
    "timeout budget",
    "error mapping",
    "seed script",
    "column order",
    "partition boundary",
    "lock ordering",
    "backpressure signal",
    "schema version",
    "idempotency key",
    "checksum check",
    "polling interval",
];

pub const QUALIFIERS: &[&str] = &[
    "under load",
    "on cold start",
    "after a failover",
    "for large workspaces",
    "in the nightly run",
    "when the queue is deep",
    "on the read replica",
    "during a rolling deploy",
];

/// Tag names. Few tags, many tasks each — the shape a real workspace converges
/// on, and the shape that makes `task_tag_rev_ix` matter.
pub const TAG_NAMES: &[&str] = &[
    "security",
    "performance",
    "tech-debt",
    "regression",
    "customer-reported",
    "needs-design",
    "blocked-external",
    "quick-win",
    "compliance",
    "accessibility",
    "flaky",
    "observability",
    "migration",
    "docs",
    "spike",
    "cost",
    "reliability",
    "api",
    "ui",
    "backend",
    "database",
    "infra",
    "onboarding",
    "billing",
    "mobile",
    "i18n",
    "privacy",
    "escalation",
    "refactor",
    "cleanup",
    "hotfix",
    "research",
    "dependency",
    "release-blocker",
    "monitoring",
    "testing",
    "tooling",
    "ux",
    "analytics",
    "support",
];

pub const TAG_COLORS: &[&str] = &[
    "#3b82f6", "#ef4444", "#10b981", "#f59e0b", "#8b5cf6", "#ec4899", "#14b8a6", "#64748b",
];

pub const ENVIRONMENTS: &[&str] = &["Production", "Staging", "Development", "QA"];

pub const MILESTONE_PREFIXES: &[&str] = &["Release", "Sprint", "Phase", "Wave", "Cycle"];

pub const COMMENT_OPENERS: &[&str] = &[
    "Reproduced on the staging environment.",
    "This overlaps with the work in the previous cycle.",
    "Adding the trace ids from the incident channel.",
    "Confirmed after the last deploy.",
    "The plan looks right, one concern below.",
    "Could not reproduce with the reference corpus.",
    "Handing this over — context is in the description.",
    "Blocked until the dependency lands.",
    "Numbers attached; the regression is real.",
    "Closing this out, the follow-up is tracked separately.",
];

pub const COMMENT_BODIES: &[&str] = &[
    "The failure only shows up once the queue is deeper than the batch size, which is why the smaller corpus never caught it.",
    "Worth checking whether the index is actually chosen here — the plan changes once the table is large enough.",
    "Left the old path in place behind a flag so the rollout can be reversed without a deploy.",
    "The timeout is inherited from the caller, so raising it here has no effect on its own.",
    "This is the third report of the same symptom; grouping them under one item.",
    "Measured before and after: the difference is inside the noise band, so it is not the cause.",
];

pub const ATTACHMENT_NAMES: &[&str] = &[
    "trace.json",
    "screenshot.png",
    "profile.svg",
    "explain-analyze.txt",
    "har-capture.har",
    "logs.tar.gz",
    "before-after.csv",
];

pub const CONTENT_TYPES: &[&str] = &[
    "application/json",
    "image/png",
    "image/svg+xml",
    "text/plain",
    "application/x-har+json",
    "application/gzip",
    "text/csv",
];

pub const PLUGIN_IDS: &[&str] = &[
    "test.casual.timetracking",
    "test.casual.chatbridge",
    "test.casual.gitlink",
    "test.casual.calendar",
    "test.casual.reporting",
];

pub const USER_AGENTS: &[&str] = &[
    "TaskForgeWeb/1.0",
    "TaskForgeCLI/0.4",
    "TaskForgeMobile/1.2 (iOS)",
    "TaskForgeMobile/1.2 (Android)",
];

/// English snowball stems for every token that can reach the search document,
/// and the stop words the parser drops.
///
/// **How this table was produced, and when it must be regenerated.** It is the
/// output of `SELECT w, to_tsvector('english', w)` run against PostgreSQL 16
/// over every token in the lists above. The generator writes `tsvector`
/// literals directly rather than calling `to_tsvector` at load time (see
/// `tasks::write_search` for why), and a hand-written lexeme that the stemmer
/// would have spelled differently is worse than useless: the document would be
/// unreachable from an ordinary `to_tsquery('english', ...)` query, and the
/// full-text gate in docs/30 would be measuring an empty result set.
///
/// Adding a word to any list above without adding it here fails
/// `every_vocabulary_token_has_a_stem`.
pub const STOPWORDS: &[&str] = &[
    "a", "after", "during", "for", "in", "is", "on", "the", "under", "when",
];

pub const STEMS: &[(&str, &str)] = &[
    ("activity", "activ"),
    ("add", "add"),
    ("anchor", "anchor"),
    ("apex", "apex"),
    ("atlas", "atlas"),
    ("attachment", "attach"),
    ("audit", "audit"),
    ("backfill", "backfil"),
    ("backpressure", "backpressur"),
    ("batch", "batch"),
    ("beacon", "beacon"),
    ("board", "board"),
    ("boundary", "boundari"),
    ("budget", "budget"),
    ("bug", "bug"),
    ("cache", "cach"),
    ("check", "check"),
    ("checksum", "checksum"),
    ("cipher", "cipher"),
    ("cold", "cold"),
    ("column", "column"),
    ("compass", "compass"),
    ("connection", "connect"),
    ("core", "core"),
    ("cursor", "cursor"),
    ("deep", "deep"),
    ("delta", "delta"),
    ("deploy", "deploy"),
    ("deprecate", "deprec"),
    ("dispatcher", "dispatch"),
    ("document", "document"),
    ("drift", "drift"),
    ("ember", "ember"),
    ("encoding", "encod"),
    ("engine", "engin"),
    ("error", "error"),
    ("failover", "failov"),
    ("fanout", "fanout"),
    ("feature", "featur"),
    ("feed", "feed"),
    ("fix", "fix"),
    ("flux", "flux"),
    ("forge", "forg"),
    ("glacier", "glacier"),
    ("halo", "halo"),
    ("harbor", "harbor"),
    ("harden", "harden"),
    ("idempotency", "idempot"),
    ("import", "import"),
    ("incident", "incid"),
    ("index", "index"),
    ("instrument", "instrument"),
    ("interval", "interv"),
    ("investigate", "investig"),
    ("iris", "iri"),
    ("job", "job"),
    ("junction", "junction"),
    ("key", "key"),
    ("keystone", "keyston"),
    ("large", "larg"),
    ("lattice", "lattic"),
    ("ledger", "ledger"),
    ("limiter", "limit"),
    ("load", "load"),
    ("lock", "lock"),
    ("loop", "loop"),
    ("lumen", "lumen"),
    ("mapping", "map"),
    ("meridian", "meridian"),
    ("migrate", "migrat"),
    ("migration", "migrat"),
    ("nightly", "night"),
    ("notification", "notif"),
    ("nova", "nova"),
    ("optimise", "optimis"),
    ("orbit", "orbit"),
    ("order", "order"),
    ("ordering", "order"),
    ("outbox", "outbox"),
    ("partition", "partit"),
    ("permission", "permiss"),
    ("pipeline", "pipelin"),
    ("plugin", "plugin"),
    ("polling", "poll"),
    ("pool", "pool"),
    ("prism", "prism"),
    ("projection", "project"),
    ("pulse", "puls"),
    ("quarry", "quarri"),
    ("queue", "queue"),
    ("rate", "rate"),
    ("read", "read"),
    ("refactor", "refactor"),
    ("remove", "remov"),
    ("renderer", "render"),
    ("replica", "replica"),
    ("request", "request"),
    ("resolver", "resolv"),
    ("retry", "retri"),
    ("rework", "rework"),
    ("rolling", "roll"),
    ("run", "run"),
    ("runner", "runner"),
    ("sandbox", "sandbox"),
    ("schema", "schema"),
    ("script", "script"),
    ("search", "search"),
    ("seed", "seed"),
    ("selection", "select"),
    ("sender", "sender"),
    ("session", "session"),
    ("signal", "signal"),
    ("size", "size"),
    ("split", "split"),
    ("start", "start"),
    ("store", "store"),
    ("stream", "stream"),
    ("summit", "summit"),
    ("task", "task"),
    ("throttle", "throttl"),
    ("timeout", "timeout"),
    ("version", "version"),
    ("vertex", "vertex"),
    ("webhook", "webhook"),
    ("workflow", "workflow"),
    ("workspaces", "workspac"),
];

/// The stem PostgreSQL's `english` configuration would produce, or `None` if
/// the token is a stop word or is not in the vocabulary.
pub fn stem(word: &str) -> Option<&'static str> {
    if STOPWORDS.contains(&word) {
        return None;
    }
    STEMS.iter().find(|(w, _)| *w == word).map(|(_, s)| *s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every token any generated title, project key, or type label can contain
    /// must be either a stop word or a known stem. Without this, adding one
    /// word to `NOUNS` silently produces documents no query can reach.
    #[test]
    fn every_vocabulary_token_has_a_stem() {
        let mut missing = Vec::new();
        let sources: Vec<&[&str]> = vec![
            VERBS,
            NOUNS,
            COMPONENTS,
            QUALIFIERS,
            PROJECT_KEYS,
            &["TASK", "BUG", "FEATURE", "INCIDENT", "REQUEST"],
        ];
        for list in sources {
            for phrase in list {
                for token in phrase.split(|c: char| !c.is_alphanumeric() && c != '-') {
                    let token = token.to_lowercase();
                    if token.is_empty() {
                        continue;
                    }
                    if !STOPWORDS.contains(&token.as_str())
                        && !STEMS.iter().any(|(w, _)| *w == token)
                    {
                        missing.push(token);
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "vocabulary tokens with no recorded stem: {missing:?} — \
             regenerate STEMS with SELECT to_tsvector('english', w)"
        );
    }

    #[test]
    fn stem_drops_stop_words() {
        assert_eq!(stem("the"), None);
        assert_eq!(stem("dispatcher"), Some("dispatch"));
        assert_eq!(stem("outbox"), Some("outbox"));
    }
}
