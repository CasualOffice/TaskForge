#!/usr/bin/env python3
"""Repository-specific security source checks.

This is deliberately narrow. Dependency advisories, secrets and the container
each have their own scanners; these checks protect TaskForge-specific rules
that a generic ruleset does not know, especially the ban on customer content
and credentials in structured logs.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE_ROOTS = (ROOT / "crates", ROOT / "tools")
TRACING = re.compile(r"tracing::(?:trace|debug|info|warn|error)!\s*\(")
SENSITIVE_FIELD = re.compile(
    r"(?:\b(?:title|description|comment_body|password|token|secret|credential|session_id)\b\s*=|"
    r"[?%]\s*(?:title|description|comment_body|password|token|secret|credential|session_id)\b(?!\.))",
    re.IGNORECASE,
)
FORBIDDEN = (
    ("danger_accept_invalid_certs", "TLS certificate verification is disabled"),
    ("danger_accept_invalid_hostnames", "TLS hostname verification is disabled"),
    ("std::process::Command", "product code may not spawn a shell command"),
)


def rust_sources() -> list[Path]:
    return sorted(
        path
        for root in SOURCE_ROOTS
        for path in root.rglob("*.rs")
        if "tests" not in path.relative_to(ROOT).parts
        and not path.name.endswith("_tests.rs")
    )


def macro_body(lines: list[str], start: int) -> str:
    """Return a bounded tracing invocation starting at ``start``."""
    body: list[str] = []
    depth = 0
    for line in lines[start : start + 40]:
        body.append(line)
        depth += line.count("(") - line.count(")")
        if depth <= 0:
            break
    return "\n".join(body)


def main() -> int:
    findings: list[str] = []
    for path in rust_sources():
        relative = path.relative_to(ROOT)
        if relative.parts[:3] == ("tools", "casual-task-lint", "src"):
            continue
        text = path.read_text(encoding="utf-8").split("#[cfg(test)]", 1)[0]
        lines = text.splitlines()
        for number, line in enumerate(lines, 1):
            code = line.strip()
            if code.startswith("//"):
                continue
            for spelling, message in FORBIDDEN:
                if spelling in code:
                    findings.append(f"{relative}:{number}: {message}")
            if TRACING.search(code):
                body = macro_body(lines, number - 1)
                if SENSITIVE_FIELD.search(body):
                    findings.append(
                        f"{relative}:{number}: tracing invocation names customer content "
                        "or credential material"
                    )

    if findings:
        print("security static checks failed:", file=sys.stderr)
        for finding in findings:
            print(f"  {finding}", file=sys.stderr)
        return 1
    print(f"security static checks: {len(rust_sources())} Rust files clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
