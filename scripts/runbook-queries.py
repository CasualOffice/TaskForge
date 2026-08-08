#!/usr/bin/env python3
"""Extract the SQL that `docs/50-RUNBOOKS.md` promises is executable.

A runbook query is read once, in an incident, by someone under time pressure who
has no way to tell a stale query from a correct one until it errors. So "✅
executable" is a promise, and this is the half of the gate that finds them.

The marker is per *step*, not per block: a heading like

    **1. Is there a backlog, and how old is it?** ✅ executable

applies to the fenced ```sql blocks that follow it, until the next bolded
heading. Blocks under a step marked `⏳ designed` are skipped — those describe
things that do not exist yet, and failing on them would make the gate a
reason to delete documentation.

Prints one query per record, separated by a NUL-delimited header so a shell can
loop over them without quoting problems.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

RUNBOOKS = Path(__file__).resolve().parent.parent / "docs" / "50-RUNBOOKS.md"

# A step heading: bolded, at the start of a line.
HEADING = re.compile(r"^\*\*(.+?)\*\*(.*)$")


def extract() -> list[tuple[str, str]]:
    """(label, sql) for every block under an executable step."""
    lines = RUNBOOKS.read_text(encoding="utf-8").splitlines()
    out: list[tuple[str, str]] = []
    executable = False
    label = "(no heading)"

    i = 0
    while i < len(lines):
        line = lines[i]
        match = HEADING.match(line)
        if match:
            heading, rest = match.groups()
            # The marker may sit on the heading line, in the bold or after it.
            executable = "✅ executable" in line
            if executable:
                label = heading.strip()
            elif "⏳" in line or rest.strip():
                # A step that is designed-not-built, or any other bolded
                # heading, ends the previous step's run of blocks.
                pass

        if line.strip() == "```sql" and executable:
            body: list[str] = []
            i += 1
            while i < len(lines) and lines[i].strip() != "```":
                body.append(lines[i])
                i += 1
            out.append((label, "\n".join(body).strip()))
        i += 1
    return out


def main() -> int:
    blocks = extract()
    if not blocks:
        print("no executable runbook queries found — the marker changed?", file=sys.stderr)
        return 1

    if "--count" in sys.argv:
        print(len(blocks))
        return 0

    for n, (label, sql) in enumerate(blocks, start=1):
        # \x00-delimited: runbook SQL contains quotes, newlines and $$ bodies.
        sys.stdout.write(f"{n}\x00{label}\x00{sql}\x00\x00")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
