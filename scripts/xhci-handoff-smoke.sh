#!/usr/bin/env bash
# Verify selected-controller handoff, plus explicit missing/ambiguous-target rejection.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
log_ok() {
    local log=$1 expected=$2
    [[ -f "$log" ]] || return 1
    if [[ "$expected" == pass ]]; then
        [[ $(rg -Fxc '[FERBER_TEST] xhci_handoff: PASS' "$log") == 1 ]] || return 1
        ! rg -qi 'panic|\[FERBER_TEST\].*: FAIL' "$log"
    elif [[ "$expected" == no-ivrs ]]; then
        rg -Fxq '[FERBER_TEST] xhci_handoff: FAIL (IVRS)' "$log" || return 1
        ! rg -Fq 'xhci_handoff: PASS' "$log"
    else
        rg -Fxq '[FERBER_TEST] xhci_resource: FAIL' "$log" || return 1
        ! rg -Fq 'xhci_handoff: PASS' "$log"
    fi
}
if [[ ${1:-} == --self-test ]]; then
    test_dir=$(mktemp -d /tmp/ferber-xhci-log.XXXXXX)
    trap 'rm -rf -- "$test_dir"' EXIT
    printf '[FERBER_TEST] xhci_handoff: PASS\n' > "$test_dir/log"
    log_ok "$test_dir/log" pass
    printf 'panic\n' >> "$test_dir/log"
    if log_ok "$test_dir/log" pass; then exit 1; fi
    printf '[FERBER_TEST] xhci_resource: FAIL\n' > "$test_dir/log"
    log_ok "$test_dir/log" fail
    printf '[FERBER_TEST] xhci_resource: FAIL\n' >> "$test_dir/log"
    log_ok "$test_dir/log" fail
    printf '[FERBER_TEST] xhci_handoff: PASS\n' >> "$test_dir/log"
    if log_ok "$test_dir/log" fail; then exit 1; fi
    printf '[FERBER_TEST] xhci_handoff: FAIL (IVRS)\n' > "$test_dir/log"
    log_ok "$test_dir/log" no-ivrs
    if log_ok "$test_dir/log" pass; then exit 1; fi
    : > "$test_dir/log"
    if log_ok "$test_dir/log" pass || log_ok "$test_dir/log" fail; then exit 1; fi
    echo 'xHCI handoff log checks passed'
    exit
fi
[[ $# == 0 ]] || { echo "usage: $0 [--self-test]" >&2; exit 2; }
ovmf_code=${OVMF_CODE:-/usr/share/edk2/x64/OVMF_CODE.4m.fd}
ovmf_vars=${OVMF_VARS:-/usr/share/edk2/x64/OVMF_VARS.4m.fd}
for required in "$ovmf_code" "$ovmf_vars"; do
    [[ -f "$required" ]] || { echo "missing firmware: $required" >&2; exit 1; }
done
for command in qemu-system-x86_64 timeout rg; do command -v "$command" >/dev/null; done
trial_dir=$(mktemp -d /tmp/ferber-xhci-handoff.XXXXXX)
smoke_passed=0
cleanup() {
    if [[ $smoke_passed == 1 ]]; then rm -rf -- "$trial_dir";
    else echo "handoff logs retained in $trial_dir" >&2; fi
}
trap cleanup EXIT
iso=$(bash "$repo_root/scripts/build-xhci-handoff.sh" qemu "$trial_dir/handoff.iso")
for scenario in present missing duplicate no-ivrs; do
    iommu=()
    [[ $scenario == no-ivrs ]] || iommu+=(-device amd-iommu,intremap=on)
    devices=()
    [[ $scenario == missing ]] || devices+=(-device qemu-xhci,id=xhci0)
    [[ $scenario != duplicate ]] || devices+=(-device qemu-xhci,id=xhci1)
    cp -- "$ovmf_vars" "$trial_dir/OVMF_VARS.fd"
    log="$trial_dir/$scenario.log"
    status=0
    timeout 15s qemu-system-x86_64 \
        -machine q35,kernel-irqchip=off -m 512M -smp 1 \
        "${iommu[@]}" "${devices[@]}" \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$ovmf_code" \
        -drive "if=pflash,format=raw,unit=1,file=$trial_dir/OVMF_VARS.fd" \
        -cdrom "$iso" -boot d -vga std -display none -monitor none -serial none -no-reboot \
        -debugcon "file:$log" -global isa-debugcon.iobase=0xe9 \
        > "$trial_dir/$scenario-qemu.log" 2>&1 || status=$?
    expected=fail
    [[ $scenario != present ]] || expected=pass
    [[ $scenario != no-ivrs ]] || expected=no-ivrs
    if [[ $status != 124 ]] || ! log_ok "$log" "$expected"; then
        cat "$trial_dir/$scenario-qemu.log" "$log" >&2
        echo "xHCI handoff smoke failed: $scenario (status $status)" >&2
        exit 1
    fi
    echo "xHCI handoff smoke passed: $scenario"
done
smoke_passed=1
