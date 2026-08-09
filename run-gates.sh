#!/usr/bin/env bash
# Serialise the Docker-backed gates against the other agents.
#
# mkdir is the atomic primitive macOS has and flock is not. The id goes INSIDE
# the lock so a stuck one can be attributed, and `rm -rf` releases it — the
# coordinator's protocol.
set -uo pipefail
cd "$(dirname "$0")"

ID="c010-attachments-$$"
until mkdir /tmp/tf-docker.lock 2>/dev/null; do sleep 15; done
echo "$ID" > /tmp/tf-docker.lock/owner
trap 'rm -rf /tmp/tf-docker.lock 2>/dev/null' EXIT
echo "=== lock acquired by $ID ==="

./scripts/check.sh 2>&1 | tail -14
check=${PIPESTATUS[0]}
echo "=== check.sh exit ${check} ==="

cargo test -p casual-task-api --all-features -- --ignored --test-threads=1 2>&1 \
  | grep -E "test result|FAILED|panicked|^error|failures:"
tests=${PIPESTATUS[0]}
echo "=== api ignored exit ${tests} ==="

rm -rf /tmp/tf-docker.lock 2>/dev/null
trap - EXIT
[ "$check" -eq 0 ] && [ "$tests" -eq 0 ]
