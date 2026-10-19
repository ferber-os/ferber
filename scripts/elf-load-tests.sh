#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-elf-load-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
rustc --edition=2021 -D warnings --test "$repo_dir/scripts/elf-load-tests.rs" -o "$test_dir/tests"
"$test_dir/tests"
