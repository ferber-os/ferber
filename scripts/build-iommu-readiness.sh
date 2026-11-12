#!/usr/bin/env bash
# Build a read-only PCI/IOMMU inventory diagnostic; this image never activates AMD-Vi.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 qemu|msi [OUTPUT_ISO]" >&2
    exit 2
fi
profile=$1
case "$profile" in
    qemu) kernel_features=iommu-readiness-probe,qemu-debug-port; loader_features=xhci-handoff-probe ;;
    msi) kernel_features=iommu-readiness-probe,xhci-target-msi; loader_features=xhci-target-msi ;;
    *) echo 'profile must be qemu or msi' >&2; exit 2 ;;
esac
output=$(realpath -m "${2:-"$repo_root/ferber-iommu-readiness-$profile.iso"}")
build_root="$repo_root/target/iommu-readiness/$profile"
CARGO_TARGET_DIR="$build_root/kernel" \
    bash "$repo_root/scripts/x86-preview.sh" --release --features "$kernel_features" >&2
(cd "$repo_root/boot/uefi" && CARGO_TARGET_DIR="$build_root/loader" \
    cargo +nightly build --release --target x86_64-unknown-uefi --features "$loader_features") >&2
bash "$repo_root/scripts/build-iso.sh" \
    "$build_root/kernel/x86_64-unknown-none/release/ferber" \
    "$build_root/loader/x86_64-unknown-uefi/release/ferber-uefi-loader.efi" "$output" >&2
printf '%s\n' "$output"
