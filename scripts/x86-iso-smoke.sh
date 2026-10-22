#!/usr/bin/env bash
# Exercise the release ISO packaging through an actual UEFI CD-ROM boot.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo_root/scripts/smoke-log.sh"
scenario=${1:-normal}
case "$scenario" in
    normal|elf|init) ;;
    *) echo "usage: $0 [normal|elf|init]" >&2; exit 2 ;;
esac
ovmf_code=${OVMF_CODE:-/usr/share/edk2/x64/OVMF_CODE.4m.fd}
ovmf_vars=${OVMF_VARS:-/usr/share/edk2/x64/OVMF_VARS.4m.fd}
for required in "$ovmf_code" "$ovmf_vars"; do
    if [[ ! -f "$required" ]]; then
        echo "missing OVMF firmware: $required" >&2
        exit 1
    fi
done
for command in qemu-system-x86_64 timeout rg; do
    if ! command -v "$command" >/dev/null; then
        echo "missing command: $command" >&2
        exit 1
    fi
done

trial_dir=$(mktemp -d /tmp/ferber-x86-iso-smoke.XXXXXX)
smoke_passed=0
cleanup() {
    if [[ "$smoke_passed" -eq 1 ]]; then
        rm -rf -- "$trial_dir"
    else
        echo "ISO smoke logs retained in $trial_dir" >&2
    fi
}
trap cleanup EXIT

cargo_args=(--release --features qemu-debug-port)
payload=
if [[ "$scenario" == elf ]]; then
    payload=$("$repo_root/scripts/build-user-elf.sh" x86_64-unknown-none release)
    cargo_args+=(--features userspace-elf-probe)
fi
if [[ "$scenario" == init ]]; then
    payload=$("$repo_root/scripts/build-user.sh" init x86_64-unknown-none release)
    cargo_args+=(--features minimal-init)
fi
# Debug-port diagnostics are used only for automated log capture.
CARGO_TARGET_DIR="$trial_dir/kernel-target" \
    "$repo_root/scripts/x86-preview.sh" "${cargo_args[@]}"
(cd "$repo_root/boot/uefi" && CARGO_TARGET_DIR="$trial_dir/loader-target" \
    cargo +nightly build --release --target x86_64-unknown-uefi)
bash "$repo_root/scripts/build-iso.sh" \
    "$trial_dir/kernel-target/x86_64-unknown-none/release/ferber" \
    "$trial_dir/loader-target/x86_64-unknown-uefi/release/ferber-uefi-loader.efi" \
    "$trial_dir/ferber.iso" "$payload"
cp -- "$ovmf_vars" "$trial_dir/OVMF_VARS.fd"

log="$trial_dir/kernel.log"
qemu_log="$trial_dir/qemu.log"
status=0
timeout 12s qemu-system-x86_64 \
    -machine q35 -m 512M -smp 1 \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$ovmf_code" \
    -drive "if=pflash,format=raw,unit=1,file=$trial_dir/OVMF_VARS.fd" \
    -cdrom "$trial_dir/ferber.iso" -boot d \
    -vga std -display none -monitor none -serial none -no-reboot \
    -debugcon "file:$log" -global isa-debugcon.iobase=0xe9 \
    >"$qemu_log" 2>&1 || status=$?
if [[ "$status" -ne 124 ]]; then
    cat "$qemu_log" >&2
    echo "QEMU ISO boot exited unexpectedly (status $status)" >&2
    exit 1
fi
if ! rg -Fq '[mem]   physical allocator: ok' "$log" \
    || ! rg -Fq 'Phase 1 boot checks complete.' "$log" \
    || ! smoke_log_ok "$log" "$scenario"; then
    cat "$log" >&2
    echo 'missing successful user-mode probe from ISO boot' >&2
    exit 1
fi
smoke_passed=1
echo "x86 QEMU ISO smoke passed: release $scenario, 512 MiB"
