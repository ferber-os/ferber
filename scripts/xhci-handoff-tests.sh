#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-xhci-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
rustc --edition=2021 -D warnings --test "$repo_root/scripts/xhci-handoff-tests.rs" -o "$test_dir/handoff-tests"
"$test_dir/handoff-tests"
