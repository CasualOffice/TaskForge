#!/usr/bin/env python3
"""Reject compile-time API error codes absent from docs/20's registry."""

from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "docs" / "20-ERROR-CODE-REGISTRY.md"
CODE = re.compile(r"TF-[A-Z]{3}-[0-9]{4}")
DECLARATION = re.compile(r'(?:Code|ErrorCode)::new\("(TF-[A-Z]{3}-[0-9]{4})"\)')


def main() -> int:
    registered = set(CODE.findall(REGISTRY.read_text(encoding="utf-8")))
    emitted: dict[str, list[Path]] = {}
    for source in sorted((ROOT / "crates").rglob("*.rs")):
        for code in DECLARATION.findall(source.read_text(encoding="utf-8")):
            emitted.setdefault(code, []).append(source.relative_to(ROOT))

    missing = {code: paths for code, paths in emitted.items() if code not in registered}
    if missing:
        print("compile-time error codes missing from docs/20:", file=sys.stderr)
        for code, paths in sorted(missing.items()):
            print(f"  {code}: {', '.join(map(str, paths))}", file=sys.stderr)
        return 1

    print(f"error registry: {len(emitted)} emitted codes are registered")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
