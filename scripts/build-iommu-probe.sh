#!/usr/bin/env bash
# Build one emulator-only DMA diagnostic; never use this image on physical hardware.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 allowed|unmapped|readonly|unmap|production [OUTPUT_ISO]" >&2
    exit 2
fi
scenario=$1
features=qemu-dma-probe,qemu-debug-port
case "$scenario" in
    allowed) ;;
    unmapped|readonly|unmap|production) features+=",qemu-dma-$scenario" ;;
    *) echo 'unknown DMA diagnostic scenario' >&2; exit 2 ;;
esac
output=$(realpath -m "${2:-"$repo_root/ferber-iommu-$scenario.iso"}")
build_root="$repo_root/target/iommu-probe/$scenario"
CARGO_TARGET_DIR="$build_root/kernel" \
    bash "$repo_root/scripts/x86-preview.sh" --release --features "$features" >&2
(cd "$repo_root/boot/uefi" && CARGO_TARGET_DIR="$repo_root/target/iommu-probe/loader" \
    cargo +nightly build --release --target x86_64-unknown-uefi) >&2
bash "$repo_root/scripts/build-iso.sh" \
    "$build_root/kernel/x86_64-unknown-none/release/ferber" \
    "$repo_root/target/iommu-probe/loader/x86_64-unknown-uefi/release/ferber-uefi-loader.efi" \
    "$output" >&2
printf '%s\n' "$output"
