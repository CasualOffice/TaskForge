#!/usr/bin/env bash
# The CI gate set, locally. See docs/15-CI-AND-RELEASE-GATES.md.
#
# Gates that need a tool you may not have — Docker, pnpm, cargo-deny — are
# skipped with a loud line rather than silently, and the summary at the end
# names every one that was skipped. CI skips nothing, so a clean run here with
# skips is not a prediction that CI will pass.
#
# Deliberately NOT run: the `image` job (docker build of the release image plus
# scripts/verify-deployment.sh). It takes minutes and rebuilds the whole
# dependency tree; run it directly before touching the Dockerfile or the deploy
# compose files.
set -euo pipefail
cd "$(dirname "$0")/.."

SKIPPED=()

run() { printf '\n\033[1m▸ %s\033[0m\n' "$*"; "$@"; }
skip() {
  printf '\n\033[33m⚠ skipped: %s (%s)\033[0m\n' "$1" "$2"
  SKIPPED+=("$1")
}

# ── Always available: the Rust gates ──────────────────────────────────────────
run cargo fmt --all -- --check
run cargo clippy --workspace --all-targets --all-features -- -D warnings
run cargo run -q -p casual-task-lint          # architecture lints
run cargo nextest run --workspace 2>/dev/null || run cargo test --workspace
# Exactly what the `docs` job runs. It used to omit both --all-features and
# RUSTDOCFLAGS, so a broken intra-doc link — a link to a private item, say —
# passed here and failed in CI. A local gate that is weaker than the real one
# is how you learn about a failure from a pull request instead of a terminal.
run env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps

# ── documentation ─────────────────────────────────────────────────────────────
run python3 scripts/check-doc-links.py
printf '\n\033[1m▸ merge-conflict markers and banned words\033[0m\n'
if git grep -nE '^(<{7}|={7}|>{7})( |$)' -- ':!*.yml'; then
  echo "merge-conflict markers found" >&2
  exit 1
fi
if git grep -nEi '\b(seamless|lossless)\b' -- 'docs/*.md' \
     ':!docs/10-PROJECT-GOAL-AND-STANDARDS.md' \
     ':!docs/16-DOCUMENTATION-MAINTENANCE.md' \
     ':!docs/17-GLOSSARY.md' \
     ':!docs/_archive/*'; then
  echo "banned word in docs (docs/16 §Writing standards)" >&2
  exit 1
fi

# ── dependency policy ─────────────────────────────────────────────────────────
if command -v cargo-deny >/dev/null 2>&1; then
  run cargo deny check bans licenses sources
else
  skip "dependency-policy" "cargo-deny not installed"
fi

# ── the database gates ────────────────────────────────────────────────────────
# Both start their own PostgreSQL 16 container and tear it down again.
if docker info >/dev/null 2>&1; then
  run ./scripts/verify-schema.sh
  run ./scripts/verify-queries.sh
else
  skip "schema" "Docker is not running"
  skip "explain-no-seq-scan" "Docker is not running"
fi

# ── the bundle budget ─────────────────────────────────────────────────────────
if command -v pnpm >/dev/null 2>&1; then
  run pnpm --dir webapp install --frozen-lockfile
  run pnpm --dir webapp typecheck
  run pnpm --dir webapp build
  run pnpm --dir webapp size-check
else
  skip "bundle-size" "pnpm not installed"
fi

if [ ${#SKIPPED[@]} -eq 0 ]; then
  printf '\n\033[32mAll local gates passed.\033[0m\n'
else
  printf '\n\033[32mLocal gates passed\033[0m, \033[33mbut %d were skipped: %s\033[0m\n' \
    "${#SKIPPED[@]}" "${SKIPPED[*]}"
  printf 'CI does not skip them.\n'
fi
