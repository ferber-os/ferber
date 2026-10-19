#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-elf-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
rustc --edition=2021 --test -D warnings "$repo_dir/scripts/elf-tests.rs" -o "$test_dir/tests"
"$test_dir/tests"
