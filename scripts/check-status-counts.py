#!/usr/bin/env python3
"""Every stated tally of tracker rows must equal the tracker.

`docs/14-EXECUTION-TRACKER.md` is the source of truth for what is `Gated`,
`Built`, `Building` or not started. Five other files state those numbers in
prose — the README, the two agent contracts, and both halves of the public site
— because the counts are the honest answer to "how finished is this?" and that
answer belongs where somebody is reading, not one link away.

Five copies of a number is five chances to be wrong, and this project has been
wrong in both directions already: the site said "built and gated" over rows the
tracker marks `Building`, and `AGENTS.md` said "no product functionality exists
yet" long after there was a product. Adding one tracker row then moved every
count at once, which is exactly the shape of drift nobody notices in review.

So the numbers are asserted, not trusted. This does **not** rewrite the prose
the way `phase-progress.py --write` rewrites the README's generated block: these
sentences differ in wording per audience, and generating them would flatten a
page written for a person into a page written for a script. What is checked is
only the digits.

Exit 0 when every file agrees with the tracker, 1 with the specific
disagreement when one does not.
"""

from __future__ import annotations

import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TRACKER = ROOT / "docs" / "14-EXECUTION-TRACKER.md"

ROW = re.compile(r"^\|\s*([FC])-(\d+)\s*\|(.+)$")
# Strongest first: a row naming several words is at the status it has reached.
STATUSES = ["Gated", "Built", "Building", "Accepted", "Designed"]


def tally() -> dict[str, Counter[str]]:
    """Rows per phase prefix, counted once each, by strongest status named."""
    counts: dict[str, Counter[str]] = {"F": Counter(), "C": Counter()}
    seen: set[str] = set()
    for line in TRACKER.read_text(encoding="utf-8").splitlines():
        match = ROW.match(line)
        if not match:
            continue
        prefix, number, rest = match.groups()
        key = f"{prefix}-{number}"
        # Items are mentioned in prose tables as well; count each row once.
        if key in seen:
            continue
        seen.add(key)
        for status in STATUSES:
            if re.search(rf"\b{status}\b", rest):
                counts[prefix][status] += 1
                break
        else:
            counts[prefix]["Accepted"] += 1
    return counts


def main() -> int:
    counts = tally()
    c = counts["C"]
    f = counts["F"]
    total = sum(c.values())
    gated, built, building = c["Gated"], c["Built"], c["Building"]
    not_started = total - gated - built - building

    # Each entry: file, a regex over its prose, and the tuple it must yield.
    # The regexes are deliberately anchored on wording, so a rewrite fails loudly
    # here rather than silently stopping checking — the failure mode that makes a
    # gate worse than no gate.
    expected: list[tuple[str, str, tuple[int, ...]]] = [
        (
            "AGENTS.md",
            r"\| 1 — core \(`C`\) \| (\d+) \| (\d+) \| (\d+) \| (\d+) \| (\d+) \|",
            (total, gated, built, building, not_started),
        ),
        (
            "AGENTS.md",
            r"\| 0 — foundation \(`F`\) \| (\d+) \| (\d+) \| (\d+) \|",
            (sum(f.values()), f["Gated"], f["Built"]),
        ),
        (
            "CLAUDE.md",
            r"Phase 0 is (\d+) `Gated` / (\d+) `Built` of (\d+); Phase 1 is (\d+) `Gated` /\s*"
            r"(\d+) `Built` / (\d+) `Building` / 1 not started, of (\d+)\.",
            (f["Gated"], f["Built"], sum(f.values()), gated, built, building, total),
        ),
        (
            "README.md",
            r"of (\d+) Phase 1 items, (\d+) carry an acceptance gate, (\d+) more are merged",
            (total, gated, built),
        ),
        (
            "site/llms.txt",
            r"Of (\d+) Phase 1\s*\nitems, \*\*(\d+) are gated\*\*, (\d+) more are merged",
            (total, gated, built),
        ),
        (
            "site/index.html",
            r"Of (\d+) Phase 1 items, (\d+) are merged and protected by an acceptance gate in CI, "
            r"(\d+) more are merged",
            (total, gated, built),
        ),
        (
            "site/index.html",
            r"Of (\d+) Phase 1\s*\n\s*items, (\d+) are gated, (\d+) more are merged",
            (total, gated, built),
        ),
        (
            "site/index.html",
            r"(\d+) of (\d+) Phase 1 items carry an\s*\n\s*acceptance gate, (\d+) more are merged",
            (gated, total, built),
        ),
    ]

    problems: list[str] = []
    for name, pattern, want in expected:
        path = ROOT / name
        text = path.read_text(encoding="utf-8")
        match = re.search(pattern, text)
        if match is None:
            problems.append(
                f"{name}: the sentence this check reads has been reworded, so it is no "
                f"longer checking anything. Update the pattern here or remove it — a gate "
                f"that stopped looking is worse than no gate.\n      looked for: {pattern}"
            )
            continue
        got = tuple(int(group) for group in match.groups())
        if got != want:
            problems.append(f"{name}: states {got}, tracker says {want}")

    if problems:
        print("tracker counts disagree with what is written:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            f"\n  tracker: Phase 0 {sum(f.values())} rows "
            f"({f['Gated']} Gated, {f['Built']} Built); "
            f"Phase 1 {total} rows ({gated} Gated, {built} Built, "
            f"{building} Building, {not_started} not started)",
            file=sys.stderr,
        )
        return 1

    print(
        f"status counts agree across 5 files: Phase 1 is {total} rows "
        f"({gated} Gated, {built} Built, {building} Building, {not_started} not started)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
