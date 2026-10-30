#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-scheduler-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
rustc +nightly --edition=2021 -D warnings --test "$repo_dir/scripts/scheduler-tests.rs" -o "$test_dir/tests"
"$test_dir/tests"
