#!/usr/bin/env python3
"""Compute per-phase progress from the execution tracker, and gate the README.

WHY THIS IS A SCRIPT AND NOT A TABLE SOMEBODY MAINTAINS

The README carried a "Phase 0" status badge for the whole of the week after
Phase 0 closed. Nobody was careless; a number written in a second place simply
has no reason to change when the first place does. A progress percentage is the
same shape of claim, updated more often, and read by people deciding whether to
try the product.

So it is derived. `docs/14-EXECUTION-TRACKER.md` is the single source — every
unit of work already has a row there, by AGENTS.md — and this script reads those
rows, computes the percentages, and in `--check` mode fails if the README
disagrees. A stale number becomes a red build instead of a quiet lie.

WHAT THE PERCENTAGE MEANS, EXACTLY

Items at `Gated` divided by items in the phase. Not effort, not lines, not
confidence: the count of work that is merged, tested, AND protected by an
acceptance gate, over the count of work the phase contains.

That is deliberately the harshest of the available readings. `Built` work is
real and is reported separately, but AGENTS.md says "done means Gated", and a
progress bar that counted anything softer would report a number the project's
own definition does not recognise.

Usage:
    scripts/phase-progress.py            # print the table
    scripts/phase-progress.py --check    # exit 1 if README.md is out of date
    scripts/phase-progress.py --write    # rewrite the README block
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TRACKER = ROOT / "docs" / "14-EXECUTION-TRACKER.md"
README = ROOT / "README.md"

BEGIN = "<!-- phase-progress:begin -->"
END = "<!-- phase-progress:end -->"

# Prefix → (phase label, roadmap description). Phases 2-4 share one tracker
# section and are reported together, because that is how they are tracked.
PHASES = [
    ("F", "0 — Foundation", "workspace, CI gates, schema + RLS, corpus, image"),
    ("C", "1 — Usable core", "auth, projects, tasks, workflow, outbox, search, **then** the web client"),
]

# A row is `| F-001 | Some item | `Gated` |` — possibly with more columns.
ROW = re.compile(r"^\|\s*([FC])-(\d+)\s*\|(.+)$")

STATUSES = ["Gated", "Built", "Building", "Accepted", "Designed"]


def statuses_in(cells: str) -> str:
    """The strongest status named in the row, or 'Accepted' if none is."""
    for status in STATUSES:
        if re.search(rf"`?{status}`?", cells):
            return status
    return "Accepted"


def tally() -> dict[str, dict[str, int]]:
    counts: dict[str, dict[str, int]] = {p: {} for p, _, _ in PHASES}
    seen: set[str] = set()
    for line in TRACKER.read_text(encoding="utf-8").splitlines():
        match = ROW.match(line)
        if not match:
            continue
        prefix, number, rest = match.groups()
        key = f"{prefix}-{number}"
        # The tracker mentions items in prose tables too; count each once.
        if key in seen or prefix not in counts:
            continue
        seen.add(key)
        status = statuses_in(rest)
        counts[prefix][status] = counts[prefix].get(status, 0) + 1
    return counts


def render() -> str:
    counts = tally()
    lines = [
        "| Phase | Delivers | Gated | Progress |",
        "| --- | --- | --- | --- |",
    ]
    for prefix, label, delivers in PHASES:
        by_status = counts[prefix]
        total = sum(by_status.values())
        gated = by_status.get("Gated", 0)
        built = by_status.get("Built", 0)
        building = by_status.get("Building", 0)
        pct = round(100 * gated / total) if total else 0
        bar = "█" * round(pct / 10) + "░" * (10 - round(pct / 10))
        detail = f"{gated}/{total}"
        if built or building:
            extra = []
            if built:
                extra.append(f"{built} built")
            if building:
                extra.append(f"{building} building")
            detail += " (" + ", ".join(extra) + ")"
        lines.append(f"| **{label}** | {delivers} | {detail} | `{bar}` {pct}% |")
    lines.append("| 2 — Administration · 3 — Extensions · 4 — Advanced | custom roles, plugins, automation, reporting | 0/— | `░░░░░░░░░░` 0% |")
    lines.append("")
    lines.append(
        "*Generated from [docs/14-EXECUTION-TRACKER.md](docs/14-EXECUTION-TRACKER.md) "
        "by `scripts/phase-progress.py`, and gated in CI so it cannot go stale. "
        "**Progress counts `Gated` items only** — merged, tested, and protected by "
        "an acceptance gate ([AGENTS.md](AGENTS.md): \"done means Gated\"). Work that "
        "is built and tested but not yet gated is shown separately rather than "
        "counted.*"
    )
    return "\n".join(lines)


def main() -> int:
    block = render()
    text = README.read_text(encoding="utf-8")

    if BEGIN not in text or END not in text:
        print(f"README.md has no {BEGIN} / {END} markers", file=sys.stderr)
        return 1

    head, rest = text.split(BEGIN, 1)
    _, tail = rest.split(END, 1)
    updated = f"{head}{BEGIN}\n{block}\n{END}{tail}"

    if "--check" in sys.argv:
        if updated != text:
            print(
                "README.md phase progress is out of date.\n"
                "The tracker moved and the README did not. Run:\n"
                "    scripts/phase-progress.py --write",
                file=sys.stderr,
            )
            return 1
        print("phase progress: README matches the tracker")
        return 0

    if "--write" in sys.argv:
        README.write_text(updated, encoding="utf-8")
        print("README.md updated")
        return 0

    print(block)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
