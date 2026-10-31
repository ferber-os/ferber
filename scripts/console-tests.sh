#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-console-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
for module in console_protocol uart16550; do
    rustc --edition=2021 -D warnings --test "$repo_root/shared/$module.rs" -o "$test_dir/$module"
    "$test_dir/$module"
done
rustc --edition=2021 -D warnings --test "$repo_root/scripts/serial-service-tests.rs" -o "$test_dir/serial-service"
"$test_dir/serial-service"
