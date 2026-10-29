#!/usr/bin/env bash
# Exercise UEFI handoff, returning syscalls, permission faults, and expected panics.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo_root/scripts/smoke-log.sh"
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

trial_dir=$(mktemp -d /tmp/ferber-x86-smoke.XXXXXX)
smoke_passed=0
declare -A payload_kernel_hash
rustc --edition=2021 -D warnings "$repo_root/scripts/boot-payload-fixture.rs" -o "$trial_dir/payload-fixture"
cleanup() {
    if [[ "$smoke_passed" -eq 1 ]]; then
        rm -rf -- "$trial_dir"
    else
        echo "smoke logs retained in $trial_dir" >&2
    fi
}
trap cleanup EXIT

(cd "$repo_root/boot/uefi" && cargo +nightly build --release --target x86_64-unknown-uefi)

run_case() {
    local profile=$1 scenario=$2 ram=$3 limit=8
    local cargo_args=(--features qemu-debug-port)
    local payload=
    local log="$trial_dir/$profile-$scenario.log"
    local qemu_log="$trial_dir/$profile-$scenario-qemu.log"
    if [[ "$profile" == release ]]; then
        cargo_args+=(--release)
    fi
    case "$scenario" in
        accounting)
            cargo_args+=(--features userspace-job-accounting-probe)
            payload=$("$repo_root/scripts/build-user.sh" job-accounting-probe x86_64-unknown-none "$profile")
            ;;
        vmo-seal|vmo-seal-write)
            if [[ "$scenario" == vmo-seal ]]; then
                cargo_args+=(--features userspace-vmo-seal-probe); program=vmo-seal-probe
            else
                cargo_args+=(--features user-vmo-seal-write-probe); program=vmo-seal-write
            fi
            payload=$("$repo_root/scripts/build-user.sh" "$program" x86_64-unknown-none "$profile")
            ;;
        vmo-map|vmo-map-write|vmo-map-execute)
            case "$scenario" in
                vmo-map) cargo_args+=(--features userspace-vmo-map-probe); program=vmo-map-probe ;;
                vmo-map-write) cargo_args+=(--features user-vmo-map-write-probe); program=vmo-map-write ;;
                vmo-map-execute) cargo_args+=(--features user-vmo-map-execute-probe); program=vmo-map-execute ;;
            esac
            payload=$("$repo_root/scripts/build-user.sh" "$program" x86_64-unknown-none "$profile")
            ;;
        vmo)
            cargo_args+=(--features userspace-vmo-probe)
            payload=$("$repo_root/scripts/build-user.sh" vmo-probe x86_64-unknown-none "$profile")
            ;;
        init|init-*)
            cargo_args+=(--features minimal-init)
            case "$scenario" in
                init) payload=$("${repo_root}/scripts/build-user.sh" init x86_64-unknown-none "$profile") ;;
                init-missing) ;;
                *) payload=$("${repo_root}/scripts/build-init-test.sh" "$scenario" x86_64-unknown-none "$profile") ;;
            esac
            ;;
        isolation) cargo_args+=(--features user-isolation-probe) ;;
        write-rx) cargo_args+=(--features user-write-probe) ;;
        execute-nx) cargo_args+=(--features user-execute-probe) ;;
        deadlock) cargo_args+=(--features cooperative-deadlock-probe) ;;
        elf|payload-missing|payload-invalid|payload-exit)
            cargo_args+=(--features userspace-elf-probe)
            case "$scenario" in
                elf) payload=$("$repo_root/scripts/build-user-elf.sh" x86_64-unknown-none "$profile") ;;
                payload-invalid) payload="$trial_dir/invalid.elf"; printf 'invalid ELF' > "$payload" ;;
                payload-exit)
                    payload="$trial_dir/exit.elf"
                    "$trial_dir/payload-fixture" x86_64-unknown-none "$payload" ;;
            esac
            ;;
    esac
    if [[ "$scenario" == exception ]]; then
        cargo_args=(--features x86-exception-probe --release)
    fi

    CARGO_TARGET_DIR="$trial_dir/kernel-target" \
        "$repo_root/scripts/x86-preview.sh" "${cargo_args[@]}"
    local kernel="$trial_dir/kernel-target/x86_64-unknown-none/$profile/ferber"
    if [[ "$scenario" == elf || "$scenario" == payload-* || "$scenario" == init || "$scenario" == init-* ]]; then
        local hash key="$profile-elf"
        [[ "$scenario" != init && "$scenario" != init-* ]] || key="$profile-init"
        hash=$(sha256sum "$kernel" | cut -d ' ' -f 1)
        if [[ -n "${payload_kernel_hash[$key]:-}" && "${payload_kernel_hash[$key]}" != "$hash" ]]; then
            echo 'payload swap rebuilt or changed the kernel' >&2
            exit 1
        fi
        payload_kernel_hash[$key]=$hash
    fi
    "$repo_root/boot/uefi/package.sh" \
        "$kernel" "$trial_dir/esp" \
        "$repo_root/boot/uefi/target/x86_64-unknown-uefi/release/ferber-uefi-loader.efi" "$payload"
    cp -- "$ovmf_vars" "$trial_dir/OVMF_VARS.fd"

    local status=0
    timeout "${limit}s" qemu-system-x86_64 \
        -machine q35 -m "${ram}M" -smp 1 \
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=$ovmf_code" \
        -drive "if=pflash,format=raw,unit=1,file=$trial_dir/OVMF_VARS.fd" \
        -drive "if=virtio,format=raw,readonly=on,file=fat:$trial_dir/esp" \
        -vga std -display none -monitor none -serial none -no-reboot \
        -debugcon "file:$log" -global isa-debugcon.iobase=0xe9 \
        >"$qemu_log" 2>&1 || status=$?

    if [[ "$status" -ne 124 ]]; then
        cat "$qemu_log" >&2
        echo "QEMU exited unexpectedly ($profile $scenario, status $status)" >&2
        exit 1
    fi
    if ! rg -Fq '[mem]   physical allocator: ok' "$log" \
        || ! rg -Fq 'Phase 1 boot checks complete.' "$log"; then
        cat "$log" >&2
        echo "missing kernel boot diagnostics ($profile $scenario)" >&2
        exit 1
    fi
    if [[ "$scenario" == exception ]]; then
        if ! rg -Fq 'F E R B E R   K E R N E L   P A N I C' "$log" \
            || rg -q 'FAIL|\[FERBER_TEST\]' "$log" \
            || ! rg -Fq 'probing x86 invalid-opcode exception...' "$log" \
            || ! rg -Fq 'detail: x86 exception vector=6 error=0x0 rip=' "$log" \
            || ! rg -Fq 'where:       src/arch/x86_64/trap.rs:' "$log"; then
            cat "$log" >&2
            echo "missing x86 exception diagnostics" >&2
            exit 1
        fi
    elif ! smoke_log_ok "$log" "$scenario"; then
        cat "$log" >&2
        echo "missing expected user-mode probe evidence ($profile $scenario)" >&2
        exit 1
    fi
    echo "x86 QEMU smoke passed: $profile $scenario, ${ram} MiB"
}

run_case debug normal 512
run_case release normal 1024
run_case release exception 512
run_case debug isolation 512
run_case release isolation 512
run_case debug write-rx 512
run_case release write-rx 512
run_case debug execute-nx 512
run_case release execute-nx 512
run_case debug deadlock 512
run_case debug elf 512
run_case release elf 512
run_case debug vmo 512
run_case release vmo 512
for profile in debug release; do
    run_case "$profile" vmo-map 512
    run_case "$profile" vmo-seal 512
    run_case "$profile" accounting 512
    run_case "$profile" vmo-seal-write 512
    run_case "$profile" vmo-map-write 512
    run_case "$profile" vmo-map-execute 512
done
run_case debug init 512
run_case release init 512
run_case debug payload-missing 512
run_case debug payload-invalid 512
run_case debug payload-exit 512
for scenario in init-missing init-malformed-ready init-exit-before-ready init-exit-after-ready init-close-bootstrap init-wrong-channel; do
    run_case debug "$scenario" 512
done
smoke_passed=1
