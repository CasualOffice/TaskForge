#!/usr/bin/env bash
#
# Refuses a commit that contains a live-looking TaskForge token.
#
# docs/40 §Tokens: "The prefix is deliberate: it makes secret-scanning tools
# (and our own pre-commit hook) able to detect a leaked token in a repository."
# The prefix existed in the document and nowhere else until now; this is the
# half that makes it worth anything.
#
# A token in a commit is not recoverable by deleting it — the object stays in
# the history and in every clone and fork. The only cheap moment is before it
# lands, which is why this is a gate and not a review note.
#
set -euo pipefail
cd "$(dirname "$0")/.."

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }

# A real token is the prefix followed by the selector, a dot, and the verifier.
# The length is what keeps this from firing on prose: `tf_pat_` in a sentence,
# or in this file, is not a token and must not fail the build — a gate that
# cries wolf gets bypassed with --no-verify.
PATTERN='tf_(pat|sat)_[0-9a-f]{24}\.[0-9a-f]{48}'

# Source and configuration only. The lock file and generated artefacts cannot
# contain a token and scanning them is how this gets slow.
hits=$(grep -rInE "$PATTERN" \
  --include='*.rs' --include='*.sql' --include='*.md' --include='*.toml' \
  --include='*.sh' --include='*.yml' --include='*.yaml' --include='*.json' \
  --include='*.ts' --include='*.tsx' --include='*.js' \
  --exclude-dir=target --exclude-dir=node_modules --exclude-dir=.git \
  --exclude='verify-no-secrets.sh' \
  . 2>/dev/null || true)

if [[ -n "$hits" ]]; then
  red "
A TaskForge token appears in the working tree:

$hits

Committing it does not just expose it — the object survives in the history and
in every clone. Revoke the token, then remove the line."
  exit 1
fi

green "no leaked credentials"
