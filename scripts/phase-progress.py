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

# The narrative half. The percentage table was generated from the first day and
# the prose beside it was not, so the prose went stale in exactly the way the
# "Phase 0" badge did — two merged pull requests of work missing from a sentence
# that begins "Landed so far". Anything in the README that states what exists is
# derived now, not just the numbers.
LANDED_BEGIN = "<!-- phase-1-landed:begin -->"
LANDED_END = "<!-- phase-1-landed:end -->"

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


def phase_one_items() -> list[tuple[str, str, str]]:
    """(id, name, status) for Phase 1 rows that are underway or done."""
    section = TRACKER.read_text(encoding="utf-8")
    start = section.index("## Phase 1 — Core (C)")
    end = section.index("## Phases 2–4", start)
    rows: list[tuple[str, str, str]] = []
    for line in section[start:end].splitlines():
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) < 3 or not re.fullmatch(r"C-\d+", cells[0]):
            continue
        status = statuses_in(cells[2])
        if status in ("Gated", "Built", "Building"):
            rows.append((cells[0], cells[1].replace("**", ""), status))
    return rows


def render_landed() -> str:
    items = phase_one_items()
    if not items:
        return "*Nothing in Phase 1 has started yet.*"

    order = {"Gated": 0, "Built": 1, "Building": 2}
    items.sort(key=lambda r: (order[r[2]], r[0]))
    bullets = [f"- **{name}** ({item}) — `{status}`" for item, name, status in items]
    gated = sum(1 for _, _, s in items if s == "Gated")
    lead = (
        f"**Phase 1 is under way, and all of it is engine.** {len(items)} items "
        f"started, {gated} gated:"
    )
    return lead + "\n\n" + "\n".join(bullets)


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


def splice(text: str, begin: str, end: str, block: str) -> str:
    if begin not in text or end not in text:
        print(f"README.md has no {begin} / {end} markers", file=sys.stderr)
        raise SystemExit(1)
    head, rest = text.split(begin, 1)
    _, tail = rest.split(end, 1)
    return f"{head}{begin}\n{block}\n{end}{tail}"


def main() -> int:
    text = README.read_text(encoding="utf-8")
    updated = splice(text, BEGIN, END, render())
    updated = splice(updated, LANDED_BEGIN, LANDED_END, render_landed())

    if "--check" in sys.argv:
        if updated != text:
            print(
                "README.md is out of date — the phase table, the landed list,\n"
                "or both. The tracker moved and the README did not. Run:\n"
                "    scripts/phase-progress.py --write",
                file=sys.stderr,
            )
            return 1
        print("README matches the tracker (phase progress and landed list)")
        return 0

    if "--write" in sys.argv:
        README.write_text(updated, encoding="utf-8")
        print("README.md updated")
        return 0

    # No flag: preview what --write would splice in, without touching the file.
    # This printed an undefined name until now — the branch nothing in CI takes,
    # which is exactly the branch that reaches a person debugging by hand.
    print(render())
    print()
    print(render_landed())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
