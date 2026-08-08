#!/usr/bin/env python3
"""Verify every internal documentation link resolves.

The design record is the source of truth (docs/16-DOCUMENTATION-MAINTENANCE.md),
and cross-references are by stable number rather than title. A link that does not
resolve means a document was renamed, moved, or never written — all of which are
build failures, not warnings.

Run: python3 scripts/check-doc-links.py
"""

from __future__ import annotations

import os
import re
import sys

LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def markdown_files() -> list[str]:
    found: list[str] = []
    for base, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in {".git", "target", "node_modules"}]
        found.extend(
            os.path.join(base, f) for f in files if f.endswith(".md")
        )
    return sorted(found)


def main() -> int:
    broken: list[tuple[str, int, str]] = []
    files = markdown_files()

    for path in files:
        with open(path, encoding="utf-8") as fh:
            for lineno, line in enumerate(fh, 1):
                for target in LINK.findall(line):
                    if target.startswith(("http://", "https://", "mailto:", "#")):
                        continue
                    resolved = os.path.normpath(
                        os.path.join(os.path.dirname(path), target.split("#")[0])
                    )
                    if not os.path.exists(resolved):
                        broken.append(
                            (os.path.relpath(path, ROOT), lineno, target)
                        )

    if broken:
        print(f"\n{len(broken)} broken internal link(s):\n", file=sys.stderr)
        for path, lineno, target in broken:
            print(f"  {path}:{lineno}  ->  {target}", file=sys.stderr)
        print(
            "\nSee docs/16-DOCUMENTATION-MAINTENANCE.md §Numbering discipline.",
            file=sys.stderr,
        )
        return 1

    print(f"documentation links: {len(files)} files checked, all resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
