#!/usr/bin/env bash
# Snapshot success means activation remains blocked, never an isolation PASS.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
log_ok() {
    local log=$1 expected=$2 marker
    [[ -f "$log" ]] || return 1
    ! rg -qi 'panic' "$log" || return 1
    if [[ "$expected" == blocked ]]; then
        [[ $(rg -Fxc '[FERBER_TEST] iommu_inventory: PASS' "$log") == 1 ]] || return 1
        [[ $(rg -Fxc '[FERBER_TEST] iommu_readiness: BLOCKED' "$log") == 1 ]] || return 1
        [[ $(rg -c '^\[FERBER_TEST\]' "$log") == 2 ]] || return 1
        ! rg -qi '\[FERBER_TEST\].*: FAIL' "$log"
    else
        case "$expected" in
            extensions) marker='[FERBER_TEST] iommu_inventory: FAIL (PCI extension snapshot)' ;;
            no-ivrs) marker='[FERBER_TEST] iommu_inventory: FAIL (IVRS)' ;;
            resource) marker='[FERBER_TEST] xhci_resource: FAIL' ;;
            *) return 1 ;;
        esac
        rg -Fxq "$marker" "$log" || return 1
        if [[ "$expected" == no-ivrs || "$expected" == extensions ]]; then
            [[ $(rg -Fxc "$marker" "$log") == 1 ]] || return 1
            [[ $(rg -Fxc '[FERBER_TEST] iommu_readiness: BLOCKED' "$log") == 1 ]] || return 1
            [[ $(rg -c '^\[FERBER_TEST\]' "$log") == 2 ]]
        else
            # OVMF may retry a failed loader; every result must be the same rejection.
            ! rg '^\[FERBER_TEST\]' "$log" | rg -Fvx "$marker" >/dev/null
        fi
    fi
}
if [[ ${1:-} == --self-test ]]; then
    test_dir=$(mktemp -d /tmp/ferber-readiness-log.XXXXXX)
    trap 'rm -rf -- "$test_dir"' EXIT
    printf '[FERBER_TEST] iommu_inventory: PASS\n[FERBER_TEST] iommu_readiness: BLOCKED\n' > "$test_dir/good"
    log_ok "$test_dir/good" blocked
    for bad in 'panic: failure' '[FERBER_TEST] iommu_inventory: PASS' '[FERBER_TEST] iommu_readiness: BLOCKED' '[FERBER_TEST] iommu_readiness: PASS' '[FERBER_TEST] xhci_handoff: PASS' '[FERBER_TEST] other: FAIL'; do
        cat "$test_dir/good" > "$test_dir/log"
        printf '%s\n' "$bad" >> "$test_dir/log"
        if log_ok "$test_dir/log" blocked; then exit 1; fi
    done
    printf '[FERBER_TEST] iommu_inventory: PASS\n' > "$test_dir/log"
    if log_ok "$test_dir/log" blocked; then exit 1; fi
    printf '[FERBER_TEST] iommu_inventory: FAIL (IVRS)\n[FERBER_TEST] iommu_readiness: BLOCKED\n' > "$test_dir/log"
    log_ok "$test_dir/log" no-ivrs
    printf '[FERBER_TEST] iommu_readiness: BLOCKED\n' >> "$test_dir/log"
    if log_ok "$test_dir/log" no-ivrs; then exit 1; fi
    printf '[FERBER_TEST] iommu_inventory: FAIL (IVRS)\n' > "$test_dir/log"
    if log_ok "$test_dir/log" no-ivrs; then exit 1; fi
    printf '[FERBER_TEST] xhci_resource: FAIL\n[FERBER_TEST] xhci_resource: FAIL\n' > "$test_dir/log"
    log_ok "$test_dir/log" resource
    printf '[FERBER_TEST] iommu_readiness: BLOCKED\n' >> "$test_dir/log"
    if log_ok "$test_dir/log" resource; then exit 1; fi
    : > "$test_dir/log"
    if log_ok "$test_dir/log" blocked || log_ok "$test_dir/log" resource; then exit 1; fi
    printf '[ready] UNKNOWN 0000:00:03.0 Chain no-read-context\n[FERBER_TEST] iommu_inventory: FAIL (PCI extension snapshot)\n[FERBER_TEST] iommu_readiness: BLOCKED\n' > "$test_dir/log"
    log_ok "$test_dir/log" extensions
    printf '[FERBER_TEST] iommu_inventory: PASS\n' >> "$test_dir/log"
    if log_ok "$test_dir/log" extensions; then exit 1; fi
    echo 'IOMMU readiness log checks passed'
    exit
fi
[[ $# == 0 ]] || { echo "usage: $0 [--self-test]" >&2; exit 2; }
ovmf_code=${OVMF_CODE:-/usr/share/edk2/x64/OVMF_CODE.4m.fd}
ovmf_vars=${OVMF_VARS:-/usr/share/edk2/x64/OVMF_VARS.4m.fd}
for required in "$ovmf_code" "$ovmf_vars"; do
    [[ -f "$required" ]] || { echo "missing firmware: $required" >&2; exit 1; }
done
for command in qemu-system-x86_64 timeout rg; do command -v "$command" >/dev/null; done
trial_dir=$(mktemp -d /tmp/ferber-iommu-readiness.XXXXXX)
smoke_passed=0
cleanup() {
    if [[ $smoke_passed == 1 ]]; then rm -rf -- "$trial_dir";
    else echo "readiness logs retained in $trial_dir" >&2; fi
}
trap cleanup EXIT
iso=$(bash "$repo_root/scripts/build-iommu-readiness.sh" qemu "$trial_dir/readiness.iso")
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
        -cdrom "$iso" -boot d -vga std -display none -monitor none \
        -serial "file:$trial_dir/$scenario-serial.log" -no-reboot \
        -debugcon "file:$log" -global isa-debugcon.iobase=0xe9 \
        > "$trial_dir/$scenario-qemu.log" 2>&1 || status=$?
    expected=resource
    # QEMU's AMD-Vi model contains an overlapping dummy MSI capability.
    # Conventional validation must reject it rather than suppressing that failure.
    [[ $scenario != present ]] || expected=extensions
    [[ $scenario != no-ivrs ]] || expected=no-ivrs
    if [[ $status != 124 ]] || ! log_ok "$log" "$expected"; then
        cat "$trial_dir/$scenario-qemu.log" "$trial_dir/$scenario-serial.log" "$log" >&2
        echo "IOMMU readiness smoke failed: $scenario (status $status)" >&2
        exit 1
    fi
    if [[ $scenario == present ]]; then
        rg -Fxq '[ready] UNKNOWN 0000:00:03.0 Chain no-read-context' "$log"
        rg -q 'conventional-only [1-9][0-9]* known-extensions-on 0 unknown-extensions 1$' "$log"
    fi
    echo "IOMMU readiness smoke passed: $scenario"
done
smoke_passed=1
