#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-vmo-syscall-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
# Use the locked dependency's production macro with host-target capability code.
bitflags_manifest=$(cargo metadata --offline --manifest-path "$repo_dir/ferber/Cargo.toml" --format-version 1 |
    python3 -c 'import json,sys; print(next(p["manifest_path"] for p in json.load(sys.stdin)["packages"] if p["name"] == "bitflags"))')
rustc --edition=2021 --crate-name bitflags --crate-type rlib "${bitflags_manifest%/Cargo.toml}/src/lib.rs" -o "$test_dir/libbitflags.rlib"
rustc --edition=2021 -D warnings --test "$repo_dir/scripts/vmo-syscall-tests.rs" --extern "bitflags=$test_dir/libbitflags.rlib" -o "$test_dir/tests"
"$test_dir/tests"
