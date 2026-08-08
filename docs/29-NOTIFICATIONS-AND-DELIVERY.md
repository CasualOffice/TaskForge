# 29 — Notifications & Delivery

`notification` was a table name in the old drafts. This is what fills it, who
receives what, and how it avoids becoming the feature users turn off.

## The design problem

Notification systems fail in one direction: **too much**. A user mutes the
product, then misses the one thing that mattered. Every decision below optimizes
for *relevance*, not coverage.

The governing rule:

> **A notification must be something the recipient would act on.**
> Everything else belongs in the activity feed, which is pull, not push.

## Reasons, not events

Users are not notified because an event occurred; they are notified because they
have a **reason** to care. Reasons are ranked, and only the highest-ranked reason
for a given event produces a notification.

| Reason | Rank | Fires when |
| --- | --- | --- |
| `MENTIONED` | 1 | `@user` in a comment or description |
| `ASSIGNED` | 2 | assigned to, or unassigned from, a task |
| `REPORTED` | 3 | a task you filed changed materially |
| `SUBSCRIBED` | 4 | you explicitly followed the task |
| `PARTICIPATED` | 5 | you commented on or edited it before |
| `TEAM` | 6 | a team-level rule matched |

**One notification per user per event, at the highest applicable rank.** Being
mentioned on a task you also reported and commented on yields one notification,
labelled `MENTIONED` — not three. This alone removes most of the noise that makes
people mute trackers.

Auto-subscription happens on: creating a task, commenting, being assigned, being
mentioned. Unsubscribing is one click and is permanent for that task, overriding
`PARTICIPATED` — but never `MENTIONED`, because a direct mention is a direct
address.

## Channels

| Channel | Latency | Default |
| --- | --- | --- |
| **In-app** | immediate (SSE) | always on |
| **Email** | batched | on for rank 1–3 |
| **Web push** | immediate | opt-in |
| **Plugin channel** (Slack, Teams, …) | per plugin | opt-in ([34](34-PLUGIN-AND-EXTENSION-ARCHITECTURE.md)) |

In-app is the source of truth: every notification lands there regardless of other
channel settings, so nothing is ever *only* in an email someone deleted.

## Preferences

Per user, per workspace, with a per-project override:

```
reason        in_app   email       push
MENTIONED     on       immediate   on
ASSIGNED      on       immediate   on
REPORTED      on       digest      off
SUBSCRIBED    on       digest      off
PARTICIPATED  on       off         off
TEAM          on       off         off
```

Email modes: `immediate` · `digest` (hourly or daily) · `off`.

Sensible defaults matter more than a rich preference screen — most users never
open it. The defaults above are the design; the screen is the escape hatch.

## Batching and suppression

Four rules, each removing a specific category of noise:

1. **Self-action suppression.** You are never notified about your own action.
   Obvious, and omitted often enough to be worth stating.
2. **Coalescing window.** Changes to the same task within 5 minutes collapse into
   one notification ("Sarah made 4 changes"). Someone editing a task for two
   minutes should not generate eight emails.
3. **Digest batching.** Non-urgent reasons accumulate into an hourly or daily
   email, ordered by project then task.
4. **Quiet hours.** Per user, timezone-aware. Immediate email and push are held
   until the window ends; in-app is unaffected.

## Delivery

```
outbox_event ──▶ notification fan-out worker
                   ├─ compute recipients + reasons
                   ├─ drop self-actions
                   ├─ resolve the highest reason per recipient
                   ├─ check preferences, quiet hours, coalescing window
                   ├─ INSERT notification              (always — in-app truth)
                   └─ enqueue per-channel delivery
```

**Recipient computation is permission-checked.** A user is never notified about a
task they cannot see — including via a mention. Mentioning someone in a private
project does not silently leak the task title into their inbox; the mention
resolves, the notification is suppressed, and the mentioning user is told the
person lacks access ([04](04-RBAC-AND-AUTHORIZATION.md)).

Delivery is at-least-once with retry and dead-lettering
([25](25-EVENTS-OUTBOX-AND-AUDIT.md)). Email delivery is deduplicated on
`(user, notification_id)` so a retried send does not produce a second email.

## Email content

- Plain text and HTML, both readable.
- Subject: `[WR-125] Task title` — stable, so mail clients thread correctly.
- `In-Reply-To` / `References` headers thread by task.
- One-click unsubscribe (RFC 8058), scoped to that *reason*, not to everything.
- Renders the change, not just "something changed."
- No task content in push notification payloads beyond the key — push payloads
  reach the OS and, potentially, a lock screen.

**Reply-to-comment by email is not in v1.** It requires inbound mail parsing,
quoted-text stripping, spoofing defenses, and per-message authentication tokens —
a meaningful attack surface for a convenience feature. Deferred, and noted here so
the omission is intentional rather than forgotten.

## The inbox

`GET /api/v1/notifications` — cursor-paginated, served by
`notification_unread_ix` ([26](26-SEARCH-INDEXING-AND-QUERY.md)).

Grouped by task, with unread first. Actions: mark read, mark all read,
unsubscribe from the task, go to task. The unread badge is a partial-index count,
not a full scan.

Read state is per user and syncs across devices over SSE — reading on a phone
clears the badge on the desktop.

## Retention

Notifications are deleted after 90 days, read or unread. They are a delivery
mechanism, not a record; the record is the activity stream, which has its own
retention ([25](25-EVENTS-OUTBOX-AND-AUDIT.md)).

## Acceptance gates

- **Dedup test** — a user with four applicable reasons for one event receives
  exactly one notification, at the highest rank.
- **Self-action test** — no notification for one's own change, across every event
  type.
- **Permission test** — mentioning a user who cannot see the task produces no
  notification and tells the mentioner.
- **Coalescing test** — 10 changes in 2 minutes produce one notification.
- **Quiet hours test** — timezone-correct across DST boundaries.
- **Digest test** — a digest contains every batched item exactly once, and none
  already sent immediately.
- **Unsubscribe test** — one-click unsubscribe affects only that reason.
