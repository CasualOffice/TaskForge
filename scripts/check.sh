#!/usr/bin/env bash
# Everything CI will run, locally. See docs/15-CI-AND-RELEASE-GATES.md.
set -euo pipefail
cd "$(dirname "$0")/.."

run() { printf '\n\033[1m▸ %s\033[0m\n' "$*"; "$@"; }

run cargo fmt --all -- --check
run cargo clippy --workspace --all-targets --all-features -- -D warnings
run cargo run -q -p casual-task-lint          # architecture lints
run cargo nextest run --workspace 2>/dev/null || run cargo test --workspace
run cargo doc --workspace --no-deps
if command -v cargo-deny >/dev/null 2>&1; then
  run cargo deny check bans licenses sources
else
  echo "⚠ cargo-deny not installed — skipped (CI does not skip it)"
fi

printf '\n\033[32mAll local gates passed.\033[0m\n'
