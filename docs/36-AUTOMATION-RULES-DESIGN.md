# 36 — Automation Rules

`automation_rule` was a table name in the old drafts with no design behind it.
This is the design: **when** something happens, **if** conditions hold, **then**
do things — evaluated with a named principal's permissions, bounded against
runaway loops.

## Shape

```json
{
  "name": "Auto-assign urgent bugs to the on-call",
  "trigger":    { "event": "task.created" },
  "conditions": { "op": "and", "clauses": [
                    { "field": "type",     "op": "eq", "value": "BUG" },
                    { "field": "priority", "op": "gte", "value": "HIGH" } ] },
  "actions":    [ { "type": "assign", "to": "@oncall" },
                  { "type": "add_tag", "tag": "triage" },
                  { "type": "comment", "body": "Assigned to on-call." } ],
  "run_as":     "<user_id>",
  "enabled":    true
}
```

`conditions` is **the same AST as the filter grammar**
([27](27-FILTER-AND-SAVED-VIEW-DSL.md)), evaluated in memory against the changed
task instead of compiled to SQL. Users get one language for search, saved views,
and automation — the alternative (a second condition syntax) is a second thing to
learn and a second thing to get subtly wrong.

## Triggers

| Trigger | Fires on |
| --- | --- |
| `task.created` | creation |
| `task.updated` | any field change; may name specific `fields` |
| `task.status.changed` | may name `from_state` / `to_state` |
| `task.assigned` / `task.unassigned` | assignment change |
| `comment.created` | new comment |
| `task.due_soon` | scheduled sweep, N hours before `due_at` |
| `task.overdue` | scheduled sweep, once per task |
| `schedule.cron` | workspace-level, minimum 15-minute interval |
| plugin-contributed | via `automation.trigger` ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)) |

Event triggers consume the outbox ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)), so
automation is **post-commit and asynchronous**. A rule cannot delay or veto the
user action that triggered it — vetoing is what `validation.transition` is for,
and it is a plugin point with a hard timeout, not an automation.

## Actions

| Action | Notes |
| --- | --- |
| `assign` / `unassign` | supports `@oncall`, `@reporter`, `@project_lead` |
| `set_field` | priority, type, milestone, environment, dates |
| `transition` | must be a valid workflow edge, checked as a real transition |
| `add_tag` / `remove_tag` | |
| `comment` | templated, with `{{task.key}}`-style interpolation |
| `create_task` | subtask or linked task from a template |
| `notify` | a user, team, or channel |
| `webhook` | to a registered endpoint |
| plugin action | via `automation.action` |

Actions run **in order**, each in its own transaction. A failing action records
the failure and stops the remaining actions for that run — it does not roll back
the ones already applied, because they are already visible to users and silently
reverting them would be worse than a partial run that is reported.

## `run_as` — the permission model

**Every rule executes as a named principal**, stored in `run_as`.

This is the single most important decision in the design. The three alternatives
are all broken:

| Alternative | Failure |
| --- | --- |
| Run as the triggering user | A rule authored by an admin executes with a guest's permissions and fails unpredictably — or worse, an unprivileged user's action performs a privileged effect. |
| Run as a system superuser | Anyone who can author a rule now has root. Rule authoring becomes a privilege-escalation primitive. |
| Run unauthorized | Automation bypasses the permission model entirely. |

With `run_as`:

- Every action is authorized exactly as if that user performed it
  ([04](04-RBAC-AND-AUTHORIZATION.md)).
- Authoring a rule requires `automation.manage` **and** the ceiling rule: you may
  only set `run_as` to yourself, or to a service account you administer. You
  cannot make a rule run as someone more privileged.
- If the `run_as` principal later loses access, the rule **fails visibly** —
  disabled after 5 consecutive permission failures, with the owner notified. It
  does not silently keep working, and it does not silently stop.
- Activity and audit records attribute the change to the principal **and** mark
  `actor_type = SYSTEM` with the rule id, so history reads
  "Automation (Auto-assign urgent bugs) assigned this to Alex" — never an
  unexplained change.

## Loop prevention

A rule whose action re-triggers its own trigger is the classic failure, and it
takes a production database with it. Four independent guards:

1. **Depth limit.** Every automation-produced event carries
   `metadata.automation_depth`. At depth 5, execution stops and the run is marked
   `depth_exceeded`.
2. **Self-trigger suppression.** A rule never processes an event its own actions
   produced, matched by rule id in the event chain.
3. **Per-task rate limit.** Maximum 20 automation runs per task per hour; beyond
   that, runs are dropped and the rule is flagged.
4. **Per-workspace budget.** 10,000 runs per hour ([21](21-API-LIMITS-AND-QUOTAS.md));
   exhaustion throttles automation only — core requests are unaffected.

All four exist because any one of them can be defeated by a sufficiently creative
rule graph. A three-rule cycle defeats self-trigger suppression; only the depth
limit catches it.

## Execution

```
outbox_event ──▶ automation matcher
                   │  index rules by (workspace, trigger.event)   ← never scan all rules
                   ├─ evaluate conditions in memory  (AST, doc 27)
                   ├─ resolve run_as → permission set
                   ├─ authorize each action
                   └─ enqueue actions, ordered, per-run correlation id
```

Rules are indexed by trigger event; a workspace with 500 rules evaluates only the
handful registered for the event that fired.

**Every run is recorded** — trigger event, rule, conditions outcome, each action's
result, duration, correlation id. Retained 30 days and surfaced in the UI. Silent
automation is unsupportable automation: the first question is always "did my rule
run, and what did it do?", and without a run log the only answer is a guess.

## Templating

`{{task.key}}`, `{{task.title}}`, `{{task.assignee.display_name}}`,
`{{actor.display_name}}`, `{{now}}`, `{{trigger.from_status}}`.

A **strict whitelist** of paths — not a general expression language, no function
calls, no arbitrary field traversal. Unknown paths render literally rather than
erroring, so a typo produces visible odd text instead of a silently failed rule.
Output is escaped at the point of use (markdown, HTML, or webhook JSON).

## Authoring UX

Automation is where "simple" products become complicated. Constraints:

- The rule builder uses the **same filter builder component** as saved views. A
  user who can filter can automate.
- **Templates first.** Auto-assign by type · notify on overdue · close subtasks
  when parent closes · tag by keyword · move to In Progress on assignment. Most
  users pick a template and change one field.
- **Dry run** — "show what this would have done over the last 7 days" before
  enabling. Automation that first runs in production is automation that first
  fails in production.
- Rules are project-scoped by default; workspace-scoped rules require
  workspace-level `automation.manage`.

## Limits

| Limit | Value |
| --- | --- |
| Rules per project | 50 |
| Rules per workspace | 500 |
| Actions per rule | 10 |
| Conditions per rule | 32 (filter grammar limit) |
| Automation depth | 5 |
| Runs per task per hour | 20 |
| Runs per workspace per hour | 10,000 |
| Action timeout | 10 s |

## Acceptance gates

- **Loop test** — a directly self-triggering rule; assert it stops at depth 5 and
  is flagged.
- **Cycle test** — three mutually triggering rules; assert termination.
- **Permission test** — a rule whose `run_as` lacks `task.transition` fails
  visibly, records the failure, and disables after 5 attempts.
- **Escalation test** — authoring a rule with a more-privileged `run_as` is
  rejected.
- **Attribution test** — every automated change is attributable in activity and
  audit to both the principal and the rule.
- **Condition parity test** — the in-memory evaluator and the SQL compiler agree
  over random ASTs ([27](27-FILTER-AND-SAVED-VIEW-DSL.md)).
- **Isolation test** — an automation storm in one workspace does not raise p95
  latency in another.
