#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-terminal-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
python3 "$repo_root/scripts/build-console-font.py" --check
rustc --edition=2021 -D warnings --test "$repo_root/shared/terminal.rs" -o "$test_dir/terminal-tests"
"$test_dir/terminal-tests"
