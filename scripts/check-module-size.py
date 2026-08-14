#!/usr/bin/env python3
"""Reject source modules that exceed TaskForge's 500-line review bound."""

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCAN_ROOTS = (ROOT / "crates", ROOT / "tools", ROOT / "webapp" / "src")
SUFFIXES = {".rs", ".ts", ".tsx", ".css"}
MAX_LINES = 500


def main() -> int:
    oversized: list[tuple[Path, int]] = []
    for scan_root in SCAN_ROOTS:
        for path in scan_root.rglob("*"):
            if not path.is_file() or path.suffix not in SUFFIXES:
                continue
            count = len(path.read_text(encoding="utf-8").splitlines())
            if count > MAX_LINES:
                oversized.append((path.relative_to(ROOT), count))

    if oversized:
        print(f"source modules must be at most {MAX_LINES} lines:")
        for path, count in sorted(oversized):
            print(f"  {path}: {count}")
        return 1

    print(f"module-size: all source modules are at most {MAX_LINES} lines")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
