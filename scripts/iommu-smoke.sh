#!/usr/bin/env bash
# Real EDU DMA translation and denial, in a fixed emulator-only PCI topology.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
log_ok() {
    local log=$1 scenario=$2
    [[ -f "$log" ]] || return 1
    [[ $(rg -Fxc "[FERBER_TEST] qemu_dma_$scenario: PASS" "$log") == 1 ]] || return 1
    [[ $(rg -c '\[FERBER_TEST\].*: PASS' "$log") == 1 ]] || return 1
    ! rg -qi 'panic|\[FERBER_TEST\].*: FAIL' "$log"
}
if [[ ${1:-} == --self-test ]]; then
    test_dir=$(mktemp -d /tmp/ferber-iommu-log.XXXXXX)
    trap 'rm -rf -- "$test_dir"' EXIT
    for scenario in allowed unmapped readonly unmap production topology; do
        printf '[FERBER_TEST] qemu_dma_%s: PASS\n' "$scenario" > "$test_dir/log"
        log_ok "$test_dir/log" "$scenario"
        if log_ok "$test_dir/log" wrong; then exit 1; fi
        printf '[FERBER_TEST] qemu_dma_%s: PASS\n' "$scenario" >> "$test_dir/log"
        if log_ok "$test_dir/log" "$scenario"; then exit 1; fi
        for bad in 'panic: unrelated failure' '[FERBER_TEST] other: FAIL' '[FERBER_TEST] other: PASS'; do
            printf '[FERBER_TEST] qemu_dma_%s: PASS\n%s\n' "$scenario" "$bad" > "$test_dir/log"
            if log_ok "$test_dir/log" "$scenario"; then exit 1; fi
        done
    done
    : > "$test_dir/log"
    if log_ok "$test_dir/log" allowed; then exit 1; fi
    echo 'IOMMU diagnostic log checks passed'
    exit
fi
[[ $# == 0 ]] || { echo "usage: $0 [--self-test]" >&2; exit 2; }
ovmf_code=${OVMF_CODE:-/usr/share/edk2/x64/OVMF_CODE.4m.fd}
ovmf_vars=${OVMF_VARS:-/usr/share/edk2/x64/OVMF_VARS.4m.fd}
for required in "$ovmf_code" "$ovmf_vars"; do
    [[ -f "$required" ]] || { echo "missing firmware: $required" >&2; exit 1; }
done
qemu_x86=${QEMU_X86:-qemu-system-x86_64}
for command in "$qemu_x86" timeout rg; do command -v "$command" >/dev/null; done
# Recent QEMU defaults to passthrough unless dma-remap is explicitly enabled.
iommu=amd-iommu,intremap=on
"$qemu_x86" --version
if "$qemu_x86" -device amd-iommu,help | rg -q 'dma-remap='; then
    iommu+=,dma-remap=on
fi
trial_dir=$(mktemp -d /tmp/ferber-iommu-smoke.XXXXXX)
smoke_passed=0
cleanup() {
    if [[ $smoke_passed == 1 ]]; then rm -rf -- "$trial_dir";
    else echo "IOMMU logs retained in $trial_dir" >&2; fi
}
trap cleanup EXIT
for scenario in allowed unmapped readonly unmap production topology; do
    build_scenario=$scenario
    [[ $scenario != topology ]] || build_scenario=allowed
    iso="$trial_dir/$build_scenario.iso"
    if [[ ! -f "$iso" ]]; then
        bash "$repo_root/scripts/build-iommu-probe.sh" "$build_scenario" "$iso" >/dev/null
    fi
    extra=()
    [[ $scenario != topology ]] || extra+=(-device edu,addr=05.0)
    cp -- "$ovmf_vars" "$trial_dir/OVMF_VARS.fd"
    log="$trial_dir/$scenario.log"
    status=0
    timeout 25s "$qemu_x86" \
        -machine q35,kernel-irqchip=off -nodefaults -m 512M -smp 1 \
        -device "$iommu" -device VGA,addr=02.0 -device edu,addr=04.0 "${extra[@]}" \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$ovmf_code" \
        -drive "if=pflash,format=raw,unit=1,file=$trial_dir/OVMF_VARS.fd" \
        -cdrom "$iso" -boot d -display none -monitor none \
        -serial "file:$trial_dir/$scenario-serial.log" -no-reboot \
        -debugcon "file:$log" -global isa-debugcon.iobase=0xe9 \
        > "$trial_dir/$scenario-qemu.log" 2>&1 || status=$?
    if [[ $status != 124 ]] || ! log_ok "$log" "$scenario"; then
        cat "$trial_dir/$scenario-qemu.log" "$trial_dir/$scenario-serial.log" "$log" >&2
        echo "IOMMU smoke failed: $scenario (status $status)" >&2
        exit 1
    fi
    echo "IOMMU smoke passed: $scenario"
done
smoke_passed=1
