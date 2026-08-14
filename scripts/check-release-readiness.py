#!/usr/bin/env python3
"""Refuse publication while known deployment release blockers remain."""

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent
blockers: list[str] = []

tracker = (ROOT / "docs/14-EXECUTION-TRACKER.md").read_text(encoding="utf-8")
d048 = re.search(r"^\| D-048 \|.*$", tracker, flags=re.MULTILINE)
if d048 is None or not re.search(r"\b(?:Accepted|Consumed)\b", d048.group(0)):
    blockers.append("D-048 is not Accepted: base-image digest policy is unresolved")

dockerfile = (ROOT / "Dockerfile").read_text(encoding="utf-8")
stages: set[str] = set()
for line in dockerfile.splitlines():
    if not line.startswith("FROM "):
        continue
    words = line.split()
    base = words[1]
    if base not in stages and "@sha256:" not in base:
        blockers.append(f"mutable Dockerfile base: {line}")
    if len(words) >= 4 and words[-2].upper() == "AS":
        stages.add(words[-1])

if not (ROOT / "deploy/upgrade.sh").exists():
    blockers.append("deploy/upgrade.sh is absent: existing-volume upgrades are unsupported")

if blockers:
    print("release publication is blocked:")
    for blocker in blockers:
        print(f"  - {blocker}")
    raise SystemExit(1)

print("release-readiness: no static deployment blockers")
