#!/usr/bin/env bash
# Serialise the Docker-backed gates against the other agents: the VM is small
# and each verification loads a corpus, so two at once get OOM-killed. mkdir is
# the atomic primitive macOS has and flock is not.
#
# Usage: ./run-gates.sh [command ...]
#   no args -> check.sh, then every ignored test
#   args    -> run that command under the lock
set -uo pipefail
cd "$(dirname "$0")"

until mkdir /tmp/tf-docker.lock 2>/dev/null; do sleep 15; done
trap 'rmdir /tmp/tf-docker.lock 2>/dev/null' EXIT
echo "=== lock acquired ==="

if [ "$#" -gt 0 ]; then
  "$@"
  status=$?
  echo "=== exit ${status} ==="
  rmdir /tmp/tf-docker.lock 2>/dev/null
  trap - EXIT
  exit "$status"
fi

./scripts/check.sh 2>&1 | tail -14
check=${PIPESTATUS[0]}
echo "=== check.sh exit ${check} ==="

cargo test --workspace -- --ignored --test-threads=1 2>&1 \
  | grep -E "test result: ok\. [1-9]|FAILED|panicked|^error|failures:"
tests=${PIPESTATUS[0]}
echo "=== cargo test exit ${tests} ==="

rmdir /tmp/tf-docker.lock 2>/dev/null
trap - EXIT
[ "$check" -eq 0 ] && [ "$tests" -eq 0 ]
