#!/usr/bin/env python3
"""The README's read-path count must equal the corpus the gate actually runs.

`scripts/verify-queries.sh` globs `tests/explain/queries/*.sql`, so the number
of read paths it proves is whatever is in that directory. The README states that
number in prose, which means it is a second place with no reason to change when
the first one does — and it drifted: the corpus reached 29 while the sentence
still said 23.

The number matters more than most. It is the one figure that tells a reader how
much of the product the no-sequential-scan guarantee actually covers, so a stale
one understates or overstates the guarantee itself.

Two other things are checked here because they are the same failure caught
earlier:

* **No duplicate numeric prefixes.** The filenames are ordered `NN-name.sql`.
  Three numbers were used twice, by two additions that did not look at each
  other. Nothing breaks — the runner globs — but a numbering scheme that does
  not number is worse than none, because it is read as an index.
* **Every file is `NN-name.sql`.** A file that does not match is a file whose
  place in the order nobody decided.

Exit 0 when they agree, 1 with the correction to make when they do not.
"""

from __future__ import annotations

import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "tests" / "explain" / "queries"
README = ROOT / "README.md"

# The sentence in README.md, with the count as the one capturing group.
CLAIM = re.compile(r"for all (\d+) read paths")
FILENAME = re.compile(r"^(\d{2})-[a-z0-9-]+\.sql$")


def main() -> int:
    queries = sorted(p.name for p in CORPUS.glob("*.sql"))
    if not queries:
        print(f"no queries found in {CORPUS.relative_to(ROOT)} — that is not a pass")
        return 1

    problems: list[str] = []

    malformed = [name for name in queries if not FILENAME.match(name)]
    if malformed:
        problems.append(
            "these are not named NN-name.sql, so their place in the order is "
            "undecided: " + ", ".join(malformed)
        )

    prefixes = Counter(
        match.group(1) for name in queries if (match := FILENAME.match(name))
    )
    for prefix, count in sorted(prefixes.items()):
        if count > 1:
            clash = ", ".join(n for n in queries if n.startswith(f"{prefix}-"))
            problems.append(f"prefix {prefix} is used {count} times: {clash}")

    text = README.read_text(encoding="utf-8")
    claim = CLAIM.search(text)
    if claim is None:
        problems.append(
            "README.md no longer contains 'for all N read paths'. If the sentence "
            "moved, move this check with it; if it went, delete this check rather "
            "than leaving a gate that passes because it stopped looking."
        )
    elif int(claim.group(1)) != len(queries):
        problems.append(
            f"README.md says {claim.group(1)} read paths; "
            f"tests/explain/queries/ holds {len(queries)}. "
            f"The gate runs all {len(queries)}."
        )

    if problems:
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print(f"read-path count: README and the corpus agree on {len(queries)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
