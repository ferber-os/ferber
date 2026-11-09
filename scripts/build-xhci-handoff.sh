#!/usr/bin/env bash
# Build a diagnostic ISO that leaves one explicitly selected xHCI controller stopped.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 qemu|msi [OUTPUT_ISO]" >&2
    exit 2
fi
profile=$1
features=xhci-handoff-probe
case "$profile" in
    qemu) kernel_features="$features,qemu-debug-port" ;;
    msi) features=xhci-target-msi; kernel_features=$features ;;
    *) echo 'profile must be qemu or msi' >&2; exit 2 ;;
esac
output=${2:-"$repo_root/ferber-xhci-$profile.iso"}
output=$(realpath -m "$output")
build_root="$repo_root/target/xhci-handoff/$profile"
CARGO_TARGET_DIR="$build_root/kernel" \
    bash "$repo_root/scripts/x86-preview.sh" --release --features "$kernel_features" >&2
(cd "$repo_root/boot/uefi" && CARGO_TARGET_DIR="$build_root/loader" \
    cargo +nightly build --release --target x86_64-unknown-uefi --features "$features") >&2
bash "$repo_root/scripts/build-iso.sh" \
    "$build_root/kernel/x86_64-unknown-none/release/ferber" \
    "$build_root/loader/x86_64-unknown-uefi/release/ferber-uefi-loader.efi" "$output" >&2
printf '%s\n' "$output"
