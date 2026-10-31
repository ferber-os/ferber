#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tool_dir=$(mktemp -d /tmp/ferber-initramfs-tool.XXXXXX)
trap 'rm -rf -- "$tool_dir"' EXIT
rustc --edition=2021 -D warnings "$repo_root/scripts/initramfs-tool.rs" -o "$tool_dir/initramfs"
"$tool_dir/initramfs" "$@"
