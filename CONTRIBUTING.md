# Contributing to TaskForge

Thank you for your interest. Please read this in full before opening a PR —
TaskForge has a design-first process, and a PR that skips it will be asked to
start over.

The full process is [docs/11-DESIGN-FIRST-PROCESS.md](docs/11-DESIGN-FIRST-PROCESS.md);
the repository rules are [docs/09-REPOSITORY-AND-CONTRIBUTION.md](docs/09-REPOSITORY-AND-CONTRIBUTION.md).
Coding agents must additionally follow [AGENTS.md](AGENTS.md).

## Before you write code

1. **Read the docs.** Start at [docs/00-README.md](docs/00-README.md). Read the
   design notes and ADRs touching your area.
2. **Check the tracker.** [docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md)
   is the live state of all work. Your change needs a row.
3. **Is there a design note?** If not, write one first. If the change trips an
   **ADR trigger**, the ADR must be **Accepted** before implementation.

> The single most common reason a PR is rejected here is that it implements a
> decision nobody wrote down. If a design question is open, raise it — do not
> resolve it silently in code.

## Setup

```sh
# Toolchain is pinned; rustup will honour rust-toolchain.toml automatically.
rustup component add clippy rustfmt

cargo install cargo-nextest --locked
cargo install cargo-deny --locked

docker compose up -d          # PostgreSQL 16 for integration tests
```

## The commands

```sh
# Fast loop
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace

# Architecture lints — the crate boundaries, SQL/HTTP placement, banned patterns
cargo run -p casual-task-lint

# Supply chain
cargo deny check bans licenses sources advisories

# Docs
cargo doc --workspace --no-deps      # RUSTDOCFLAGS="-D warnings" in CI

# Everything CI will run
./scripts/check.sh
```

Integration tests use `testcontainers` and need a working Docker socket. They are
skipped with a clear message if Docker is unavailable — but they are **not**
optional in CI.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/), with the tracker ID
in the footer:

```
feat(authz): add constraint evaluation to the resolver

Implements assignee_is_actor and reporter_is_actor per docs/04.
Unconstrained grants take precedence over constrained ones.

Refs: C-003
ADR: ADR-004
Signed-off-by: Your Name <you@example.com>
```

Types: `feat` `fix` `docs` `refactor` `perf` `test` `chore` `build` `ci`.

`Signed-off-by` is required — see [Licensing](#licensing).

## Pull requests

**One coherent capability per PR.** A PR that adds a feature *and* refactors a
module is two PRs; the refactor hides the feature's real diff.

Include in the description:

- What and why, linked to the design note.
- The tracker ID.
- The ADR reference, if a trigger fired.
- Which acceptance gates now cover it.
- **What was not done, and why.** A PR implementing four of five acceptance
  criteria is fine. A PR that *silently* implements four of five is not.

### Review checklist

- Does it match the design note? If it diverges, is the note updated in this PR?
- Is every new query path indexed and asserted ([docs/26](docs/26-SEARCH-INDEXING-AND-QUERY.md))?
- Is every mutation authorized and tenant-scoped?
- Does it write activity, audit, and outbox in the same transaction?
- Are new error codes registered ([docs/20](docs/20-ERROR-CODE-REGISTRY.md))?
- Are new inputs bounded ([docs/21](docs/21-API-LIMITS-AND-QUOTAS.md))?
- Do tests cover the failure modes, not only the happy path?
- Does it add a user-facing noun? → ADR required
  ([docs/17](docs/17-GLOSSARY.md)).

## Definition of done

Done means **`Gated`**, not `Built`:

1. Merged, all CI gates green.
2. Tests cover the behaviour **and its failure modes**.
3. An acceptance gate protects it from regression.
4. Design note, ADR register, support matrix, and tracker updated.
5. Error codes, limits, and filter fields registered where applicable.

Code that passes its own tests but has no gate protecting it will regress
unnoticed. That is why the tracker distinguishes the two.

## Code standards

- Rust 2024; `unsafe_code = "forbid"`; clippy clean at `-D warnings`.
- **All SQL is compile-checked** via `sqlx::query!`, and lives only in
  `casual-task-persistence`.
- Errors are typed and carry a registry code. `unwrap()` outside tests needs a
  comment proving it cannot panic.
- No `#[allow(...)]` without a comment explaining why.
- Public items are documented.
- Property tests where an invariant exists — "adding a grant never removes a
  permission" is worth more than fifty hand-written cases.

## Documentation standards

From [docs/16-DOCUMENTATION-MAINTENANCE.md](docs/16-DOCUMENTATION-MAINTENANCE.md):

- **One owner per fact.** Link; do not restate.
- **State costs.** Every trade-off has a losing side; name it.
- **Date external claims.** Undated research is a guess after six months.
- **Banned words**: "simply", "just", "obviously", "seamless", "lossless". Each
  hides an unverified claim.

## Clean-room constraint

**Binding on every contribution.**

TaskForge must not contain source code, database schemas, templates, assets,
strings, or documentation copied or adapted from OrangeScrum or any other
tracker.

Permitted: studying published behaviour and public documentation (recorded with
sources and dates); implementing well-known patterns from general engineering
knowledge; interoperating with published formats and public APIs.

Not permitted: reading another tracker's source and writing the equivalent here;
copying a schema, permission table, UI template, or icon set; porting code
through a translator, an LLM, or a paraphrase.

If you have worked on a competing product, please say so before contributing to
the corresponding area. This protects you as much as it protects the project.

## Licensing

Contributions are licensed **Apache-2.0**. We use the
[Developer Certificate of Origin](https://developercertificate.org/) — add
`Signed-off-by:` to each commit (`git commit -s`). There is no CLA.

Dependencies must be Apache-2.0-compatible; `cargo-deny` fails the build
otherwise. **No copyleft dependencies**, however convenient.

## Security

Do **not** open a public issue for a security problem. See
[SECURITY.md](SECURITY.md).

## Code of conduct

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
