#!/usr/bin/env python3
"""Populate the dev stack with data that looks like a team's, not a fixture's.

# Why this is not a fixture dump

The first version of this seeder created twenty-five tasks with a title and
nothing else. Every board card rendered the same: no priority, no owner, no
date, no description. The client was fine; the data was lifeless, and a board
of identical grey cards is indistinguishable from a broken board.

So this writes what a real backlog has — a spread of priorities and types,
dates in the past and the future, several people, comments, and work at every
stage of the workflow. It is the difference between "the API accepts a task"
and "you can look at this and judge the product".

# Everything goes through the HTTP API

Not INSERTs. Going through the real endpoints means the data carries the owner
grant (D-054), the activity and audit rows ADR-006 requires, and the outbox
events search and SSE consume. Rows inserted behind the API look identical in
the tables and behave differently in every feature built on them.

# The workflow is a chain

Reaching Done means walking Backlog -> Todo -> In Progress -> Done one
transition at a time. That is the state machine doing its job (docs/23), so the
seeder walks it rather than trying to jump and reporting the refusal as an
error.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import urllib.parse
import uuid
from datetime import datetime, timedelta, timezone

API = os.environ.get("TF_API", "http://127.0.0.1:8080")
EMAIL = os.environ.get("TF_DEV_EMAIL", "demo@taskforge.test")
PASSWORD = os.environ.get("TF_DEV_PASSWORD", "taskforge demo password")
JAR = os.environ.get("TF_COOKIE_JAR", ".dev/cookies.txt")

NOW = datetime.now(timezone.utc)


def curl(args: list[str]) -> str:
    out = subprocess.run(["curl", "-sS", *args], capture_output=True, text=True)
    return out.stdout


class Client:
    """The four headers the API requires, in one place.

    A seeder that forgot `Idempotency-Key` would be told so (TF-IDM-0003), and
    a seeder that forgot `If-Match` on a transition likewise (TF-CNC-0002).
    Both requirements exist so a retried write cannot silently duplicate or
    clobber, and a script is exactly the caller that retries blind.
    """

    def __init__(self) -> None:
        body = curl([
            "-c", JAR, "-H", "content-type: application/json",
            "-d", json.dumps({"email": EMAIL, "password": PASSWORD}),
            f"{API}/api/v1/auth/login",
        ])
        parsed = json.loads(body)
        if "csrf_token" not in parsed:
            sys.exit(f"login failed: {body[:300]}")
        self.csrf = parsed["csrf_token"]
        self.workspace: str | None = None

    def _headers(self) -> list[str]:
        h = ["-b", JAR, "-H", f"x-csrf-token: {self.csrf}"]
        if self.workspace:
            h += ["-H", f"x-workspace-id: {self.workspace}"]
        return h

    def get(self, path: str) -> dict:
        return json.loads(curl([*self._headers(), f"{API}{path}"]))

    def post(self, path: str, body: dict, version: int | None = None) -> dict:
        args = [
            *self._headers(), "-X", "POST",
            "-H", "content-type: application/json",
            "-H", f"Idempotency-Key: {uuid.uuid4().hex}",
        ]
        if version is not None:
            args += ["-H", f'If-Match: "{version}"']
        args += ["-d", json.dumps(body), f"{API}{path}"]
        return json.loads(curl(args))

    def patch(self, path: str, body: dict, version: int) -> dict:
        args = [
            *self._headers(), "-X", "PATCH",
            "-H", "content-type: application/json",
            "-H", f'If-Match: "{version}"',
            "-d", json.dumps(body), f"{API}{path}",
        ]
        return json.loads(curl(args))


def iso(days: int) -> str:
    return (NOW + timedelta(days=days)).isoformat().replace("+00:00", "Z")


# (title, type, priority, days_from_now_due_or_None, description, stage)
# stage: 0 backlog, 1 todo, 2 in progress, 3 done
BACKLOG: dict[str, list[tuple]] = {
    "WEB": [
        ("Board drag and drop drops the card in the wrong column", "BUG", "URGENT", -1,
         "Dragging from Planned to Active lands the card one column right of the drop target. "
         "Reproduces at viewport widths under 1280px, so it is probably the column offset "
         "calculation using the scroll container rather than the board.", 2),
        ("Task drawer loses unsaved comment text on close", "BUG", "HIGH", 2,
         "Type a comment, press Escape, reopen the drawer: the text is gone. "
         "Draft should survive a close, or the close should warn.", 2),
        ("Command palette should search tasks, not just commands", "FEATURE", "HIGH", 5,
         "Cmd-K currently matches command names. It should also match task keys and titles, "
         "with tasks ranked below exact command matches.", 1),
        ("Keyboard navigation audit for the board", "TASK", "MEDIUM", 9,
         "Every card must be reachable and movable by keyboard alone. dnd-kit's keyboard "
         "sensor is wired; this is the audit that proves it end to end.", 1),
        ("Bundle budget review before release", "TASK", "MEDIUM", 14,
         "ADR-024 budgets 200 KiB gzip for the initial shell. We are at 131.8 KiB. "
         "Confirm the report attributes every initial chunk before we add the design system.", 0),
        ("Dark mode contrast fails on secondary text", "BUG", "MEDIUM", 4,
         "Muted text on the card background measures 3.9:1. WCAG AA wants 4.5:1 for body text.", 1),
        ("Empty states are blank rather than helpful", "TASK", "LOW", None,
         "A project with no tasks shows an empty column. It should say what to do next.", 0),
        ("Error boundary copy says 'Something went wrong'", "TASK", "LOW", None,
         "Every refusal already carries a docs/20 code and a request id. Show them — "
         "a user who can quote a request id gets help in one round trip instead of five.", 0),
        ("Virtualised list jumps when a row resizes", "BUG", "MEDIUM", 7,
         "Long titles wrap to two lines and the estimated row height is fixed, so the "
         "scroll position drifts as you scroll.", 0),
    ],
    "API": [
        ("Rate limit the write classes", "TASK", "HIGH", 1,
         "Login is limited per IP. Reads, writes, search, bulk and invites are not. "
         "docs/21 fixes the numbers; the limiter is in place and only wired to auth.", 2),
        ("Attachment virus scanning is unimplemented", "TASK", "HIGH", 6,
         "The pipeline is fail-closed: with no scanner an attachment stays PENDING and is "
         "never downloadable. Correct, and it means attachments do not work out of the box.", 2),
        ("Search relevance puts closed tasks above open ones", "BUG", "MEDIUM", 3,
         "ts_rank alone ignores state. Recency and open-ness should both weigh in.", 1),
        ("Outbox dead-letter alerting", "TASK", "MEDIUM", 11,
         "dlq_depth is exported and nothing alerts on it. A dead-lettered event is a "
         "notification a user never got.", 1),
        ("Permission explain endpoint", "FEATURE", "MEDIUM", None,
         "'Why can't I close this?' is the most common support question in any tracker. "
         "POST /permissions/explain answers it with the actual contributing grants.", 3),
        ("Session revocation takes up to 15 seconds to close a stream", "BUG", "HIGH", 2,
         "Revoking a session closes its SSE stream on the next revalidation tick. "
         "The window is the tick interval; LISTEN/NOTIFY is the way out.", 1),
        ("Cross-workspace token check", "INCIDENT", "URGENT", -3,
         "A token issued for workspace A authenticated against workspace B. "
         "Found by the adversarial audit, fixed, and this is the regression test row.", 3),
    ],
    "ONB": [
        ("Set up the development environment", "TASK", "HIGH", -2,
         "scripts/dev-up.sh takes an empty machine to a login screen. "
         "If it does not work for you, that is the bug.", 3),
        ("Read docs/02-ARCHITECTURE.md", "TASK", "MEDIUM", 1,
         "Start here. AGENTS.md is the contract; 02 is the shape.", 3),
        ("Get database access", "TASK", "MEDIUM", 2,
         "You connect as taskforge_app, never as the owner. A superuser bypasses every "
         "RLS policy and the API refuses to start as one.", 3),
        ("Pair on the outbox", "TASK", "MEDIUM", 5,
         "Domain change, activity, audit and outbox in one transaction. "
         "The dispatch loop never holds a transaction across consumer I/O.", 1),
        ("Ship a first change", "TASK", "LOW", 8,
         "Small, tested, behind a gate. 'Done' means Gated.", 0),
        ("Write your first runbook entry", "TASK", "LOW", 12,
         "Every runbook query in docs/50 is executed by CI. Prose that cannot run rots.", 0),
        ("Review the threat model", "TASK", "MEDIUM", 15,
         "docs/07 §Review. It was written by an agent and asks to be countersigned "
         "by a human before the Phase 1 gate.", 0),
    ],
    "OPS": [
        ("Backup restore drill", "TASK", "HIGH", 3,
         "A backup nobody has restored is a hypothesis. Restore to a scratch instance "
         "and diff row counts against the source.", 1),
        ("Upgrade rehearsal", "TASK", "MEDIUM", 10,
         "expand -> migrate -> contract, docs/52 §Upgrades. Migrations currently run on "
         "first start only; upgrading an existing volume is manual.", 0),
        ("On-call handover doc", "TASK", "MEDIUM", 6,
         "What pages, what it means, what to do first, and who to wake.", 1),
        ("Capacity review at 2M tasks", "TASK", "LOW", 20,
         "p95 read under 150 ms against the reference corpus, no sequential scans.", 2),
        ("Disk filled during a corpus load", "INCIDENT", "URGENT", -5,
         "The 2M-row corpus is 10.2 GiB of COPY text and the loader was OOM-killed. "
         "Check free space before running reference scale.", 3),
    ],
}

COMMENTS = [
    "Reproduced on my machine — same offset, same viewport width.",
    "I think this is the same root cause as the scroll drift. Linking them.",
    "Picked this up. Should have something to look at this afternoon.",
    "Blocked on the scanner decision — leaving it in progress but not working on it.",
    "Fixed and merged. Leaving open until the regression test lands.",
    "Can we split this? The audit is a day and the fixes are a week.",
]


def main() -> int:
    client = Client()

    workspaces = client.get("/api/v1/workspaces")["data"]
    demo = next((w for w in workspaces if w["slug"] == "demo"), None)
    if demo is None:
        demo = client.post("/api/v1/workspaces", {"name": "Demo", "slug": "demo"})
    client.workspace = demo["id"]
    print(f"workspace {demo['slug']}")

    existing = {p["key"]: p for p in client.get("/api/v1/projects")["data"]}
    names = {"WEB": "Web client", "API": "Platform", "ONB": "Onboarding", "OPS": "Operations"}
    projects = {}
    for key, name in names.items():
        if key in existing:
            projects[key] = existing[key]
        else:
            projects[key] = client.post("/api/v1/projects", {"name": name, "key": key})

    # The workflow's statuses, read from the API rather than guessed. Board
    # columns are permanent states, but a transition needs a status id.
    workflow = client.get(f"/api/v1/workflows/{projects['WEB']['workflow_id']}")
    by_state: dict[str, str] = {}
    for status in workflow["statuses"]:
        by_state.setdefault(status["state"], status["id"])
    chain = [by_state["PLANNED"], by_state["ACTIVE"], by_state["COMPLETED"]]

    already = {t["title"] for t in client.get("/api/v1/tasks?limit=100")["data"]}
    made = moved = commented = 0

    for key, rows in BACKLOG.items():
        project = projects[key]["id"]
        for title, kind, priority, due, description, stage in rows:
            if title in already:
                continue
            body = {"title": title, "type": kind, "priority": priority,
                    "description": description}
            if due is not None:
                body["due_at"] = iso(due)
            task = client.post(f"/api/v1/projects/{project}/tasks", body)
            if "error" in task:
                print(f"  ! {title[:40]}: {task['error']['message'][:80]}")
                continue
            made += 1

            version = task["version"]
            for status in chain[:stage]:
                moved_task = client.post(
                    f"/api/v1/tasks/{task['id']}/transitions",
                    {"to_status_id": status}, version=version)
                if "error" in moved_task:
                    print(f"  ! move {title[:30]}: {moved_task['error']['message'][:70]}")
                    break
                version = moved_task.get("version", version + 1)
            else:
                if stage:
                    moved += 1

            # Comments on the work that is actually moving, which is where a
            # real thread would be.
            if stage in (1, 2):
                for text in COMMENTS[made % 3: made % 3 + 2]:
                    client.post(f"/api/v1/tasks/{task['id']}/comments",
                                {"body": text, "mentions": []})
                    commented += 1

    print(f"{made} tasks, {moved} advanced through the workflow, {commented} comments")

    counts: dict[str, int] = {}
    for task in client.get("/api/v1/tasks?limit=100")["data"]:
        counts[task["state"]] = counts.get(task["state"], 0) + 1
    print("board:", ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
