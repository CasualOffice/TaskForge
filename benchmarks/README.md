# benchmarks/ — committed latency baselines

The committed reference points that `tools/casual-task-loadtest` compares
against. See
[docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md](../docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md)
§Measurement, which is the design this directory implements:

> CI fails on a **>10% regression** against the committed baseline, not on
> absolute numbers — absolute thresholds fail on CI noise and get disabled,
> which is worse than no gate.

## The three rules

**1. A baseline belongs to one named environment.** `docs/30` §Reference
environment names the machine gates run on (8 vCPU / 32 GB / NVMe PostgreSQL)
and states that measurements from anywhere else "are recorded with the
environment attached and are not comparable to these gates." The harness
enforces that: comparing a report from environment A against a baseline from
environment B is refused, not scaled, not warned about. The same applies to the
corpus — `docs/30` runs the full corpus nightly and a reduced one per PR, and
those two never gate each other.

**2. Baselines are committed.** A baseline stored anywhere but the repository is
a baseline nobody can review. It is a normal reviewed file, and its diff is the
record of every performance change the project has accepted.

**3. Changing a baseline requires the PR to say why.** The gate has exactly two
outcomes when a number moves past tolerance: the change is a regression and
should be fixed, or the number is now correct and the baseline should move. The
second is legitimate and is not discouraged — but it is a decision, so it is
written down. A PR that moves a baseline must:

- update the `baseline.justification` field in the file with the reason the new
  number is acceptable (what changed, why the cost is worth it, and what the
  number would have to reach before it is not);
- set `baseline.recordedBy` to the tracker item or PR;
- state the same thing in the PR description, so the reviewer approves the
  slowdown rather than the diff.

"Rebaselined to make CI green" is not a justification. Neither is a bare commit
that moves the numbers with no field changed — that is how a gate becomes
decoration.

## Files

| File | Purpose |
| --- | --- |
| `<environment>.<corpus-scale>.json` | one committed baseline |
| `reference-8vcpu-32gb.reference.json` | **placeholder** — see below |
| `smoke-local.smoke.json` | a real measurement on a developer laptop; **not a gate** |
| `smoke-corpus.sql` | a reduced, disposable corpus; **not** the reference corpus |

### The reference baseline is a placeholder, and cannot be passed

`reference-8vcpu-32gb.reference.json` carries the right shape and no data. Every
measured number in it is zero, and `baseline.status` is `placeholder`. The
harness refuses a placeholder before it reads a number, and separately refuses a
baseline whose p95 is zero, so there is no arrangement of arguments under which
a run passes against it. That is deliberate: a gate that cannot run must fail
loudly rather than pass quietly.

One of the two things it was waiting for now exists.

1. ~~**The reference corpus**~~ — **landed (F-006).**
   `tools/casual-task-seed --scale reference` generates it deterministically:
   2,000,000 tasks / 200 projects / 500 users / 20.5 M activity events,
   38,981,941 rows and 10.2 GiB of `COPY` text in 18.2 s, and it has been loaded
   into a PostgreSQL 16 and measured against end to end. Byte-identity across
   runs is gated by `tools/casual-task-seed/tests/determinism.rs`.
2. **The reference environment** — the machine in `docs/30`. Still missing. A
   number measured on a laptop cannot be committed under that environment's
   name, so the placeholder stays until the gate runs somewhere it can mean
   something.

F-007 is therefore still **Built, not Gated**, but for one reason instead of
two: the harness, the corpus, and the comparison gate all work and are tested,
and no CI job compares numbers because there is no environment whose numbers
would be comparable. `docs/15` §Pending gates records it.

**`benchmarks/smoke-corpus.sql` is kept, not deleted.** The earlier plan was to
delete it once F-006 landed. That would orphan `smoke-local.smoke.json`, which
is a committed measurement *of that corpus* — and a measurement whose corpus no
longer exists cannot be checked by anyone. It stays as provenance. New work
should use `casual-task-seed --scale small`, which is the same idea done
properly.

## Running it

Measure:

```sh
cargo run -p casual-task-loadtest -- run \
  --environment reference-8vcpu-32gb \
  --corpus-scale reference \
  --generated-at "$(git log -1 --format=%cI)" \
  --dsn "$TASKFORGE_APP_DSN" \
  --iterations 1000 --warmup 100 \
  --out /tmp/report.json
```

Gate:

```sh
cargo run -p casual-task-loadtest -- compare \
  --baseline benchmarks/reference-8vcpu-32gb.reference.json \
  --report   /tmp/report.json
```

Exit codes: `0` no regression · `1` a case regressed past tolerance · `2` the
comparison could not be performed at all (wrong environment, wrong corpus scale,
schema-version change, placeholder baseline, a case the report dropped). The
last is a distinct code on purpose — "the gate says no" and "the gate did not
run" are different failures and want different responses.

`--generated-at` is required rather than read from the clock so that rerunning
the same measurement produces a byte-identical file; the field is provenance and
is never compared. `--dsn` must be the non-superuser `taskforge_app` role: RLS is
inert for a superuser (`migrations/0012`), so a superuser measurement omits a
predicate every real query pays. The harness refuses to run as one.

When `psql` is not on the host, route through a container the way
`scripts/verify-schema.sh` does:

```sh
--psql "docker exec -i tf-loadtest psql" --dsn "postgres://taskforge_app:...@127.0.0.1/tf"
```

## What a baseline number means — and does not

Phase 0 has no API process, so **every number in this directory is a database
round trip and nothing else.** Excluded: HTTP and TLS, routing and middleware,
authorization resolution, JSON serialization, connection-pool checkout, the
per-transaction `SET LOCAL` that scopes a tenant, concurrency of any kind, and
all four write rows of the `docs/30` latency table. Each exclusion is listed in
the report's `notMeasured` array with the phase it arrives in; run
`cargo run -p casual-task-loadtest -- cases` to read the same list.

A number here is therefore a **floor** for the corresponding `docs/30` target,
not an estimate of it. The `roundtrip_floor` case measures a query that does no
work, so every other case can be read as "this much above the protocol floor" —
on a noisy machine that floor can be most of the number, which is the fastest
way to notice the machine is not fit to gate on.

## Reproducing the smoke measurement

```sh
docker run -d --name tf-loadtest -e POSTGRES_USER=tf -e POSTGRES_PASSWORD=tf \
  -e POSTGRES_DB=tf postgres:16-alpine
for f in migrations/*.sql; do docker exec -i tf-loadtest psql -U tf -d tf -v ON_ERROR_STOP=1 -q < "$f"; done
docker exec -i tf-loadtest psql -U tf -d tf -c "ALTER ROLE taskforge_app WITH LOGIN PASSWORD 'apppw';"
docker exec -i tf-loadtest psql -U tf -d tf -v ON_ERROR_STOP=1 -q < benchmarks/smoke-corpus.sql
cargo run -p casual-task-loadtest -- run \
  --environment smoke-local --corpus-scale smoke \
  --generated-at 2026-08-08T00:00:00Z \
  --psql "docker exec -i tf-loadtest psql" \
  --dsn "postgres://taskforge_app:apppw@127.0.0.1/tf" \
  --search-term payment --iterations 1000 --warmup 100 --out /tmp/report.json
```

The committed `smoke-local.smoke.json` came from exactly that. It is kept as a
worked example of the file format, and because two of its numbers were worth
carrying into F-006 and F-008 rather than losing:

- **`my_work_assigned` planned a sequential scan on `task`** at 100,000 rows —
  the planner preferred a hash join over the 1,667 assignee rows to a nested
  loop on `task_pkey`. `docs/26` NFR-5 forbids exactly that on a tenant-scale
  table.

  **Answered by F-008, partly.** The `explain-no-seq-scan` gate now plans all
  five My Work queries against its ~109k-task corpus and every one of them is
  index-served — `task_assignee_user_ix` into `task_pkey`, which is the shape
  NFR-5 wants. So the smoke result was an artefact of that corpus, not a
  property of the query.

  It is *partly* answered because the `EXPLAIN` gate runs at ~109k tasks, not at
  2,000,000. The probe constants in `tests/explain/probes.sql` follow
  `seed.sql`'s id scheme, so pointing the gate at a `casual-task-seed` corpus
  with `--data-loaded` addresses rows that do not exist and plans for an empty
  result. Planning the suite at reference scale needs the probes to be derived
  from the corpus rather than hard-coded — worth doing, and **D-043 is why it
  matters more than it looked**: a query that is index-served at 109k rows is
  not necessarily index-served at 2M, and one of them is not.
- **`full_text_search` spent its time ranking, not matching.** The GIN index
  found 12,500 matches in under a millisecond; `ORDER BY ts_rank_cd DESC` over
  all of them took the remaining ~50 ms. That is the cost shape `docs/26`'s
  ADR-014 tripwire is watching (p95 > 300 ms at the reference corpus, after
  tuning), and it scales with match count rather than corpus size.

  **Confirmed at reference scale, and the shape is worse than predicted.**
  Ranking does dominate: for a 6%-selective term the GIN finds 125,595 matches
  in ~6 ms and everything after that is heap fetch, `ts_rank_cd`, and a sort
  that spills to disk at the default `work_mem`. But the larger finding is
  D-043 — under RLS, as `taskforge_app`, the GIN is not used at all and the
  query sequentially scans `task_search`. No timing from this session is
  committed: the machine was concurrently loading a 10 GiB corpus, so the
  numbers move by 3× between runs. The *plans* are not load-dependent, and they
  are what D-043 rests on.

A third number is worth reading as a warning rather than a result:
`activity_page` reports `rowsReturned: 3`, because the smoke corpus writes
exactly three events per task. It is measuring a three-row page against a
`LIMIT 50` query, so it exercises the index but not the page. Every case carries
`rowsReturned` for exactly this reason — a query that finds nothing, or almost
nothing, gets faster, and without the row count that reads as an improvement.

Neither number is a verdict — the machine is a Docker Desktop VM whose
`roundtrip_floor` p95 is about eight times its own p50, which is precisely the
kind of noise the relative-comparison rule exists to survive.
