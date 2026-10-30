#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-bundle-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
for module in boot_bundle service_protocol; do
    rustc --edition=2021 -D warnings --test "$repo_root/shared/$module.rs" -o "$test_dir/$module"
    "$test_dir/$module"
done
