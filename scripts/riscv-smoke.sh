#!/usr/bin/env bash
# Exercise returning syscalls, permission faults, and cooperative deadlock detection.
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$repo_dir/scripts/smoke-log.sh"
trial_dir=$(mktemp -d /tmp/ferber-riscv-smoke.XXXXXX)
smoke_passed=0
declare -A payload_kernel_hash
rustc --edition=2021 -D warnings "$repo_dir/scripts/boot-payload-fixture.rs" -o "$trial_dir/payload-fixture"
cleanup() {
    if [[ "$smoke_passed" -eq 1 ]]; then
        rm -rf -- "$trial_dir"
    else
        echo "smoke logs retained in $trial_dir" >&2
    fi
}
trap cleanup EXIT

run_case() {
    local profile=$1 scenario=$2 size=$3 harts=$4 limit=8
    local log="$trial_dir/$profile-$scenario-${size}M-${harts}h.log"
    local qemu_log="$trial_dir/$profile-$scenario-${size}M-${harts}h-qemu.log"
    local cargo_args=(--target riscv64gc-unknown-none-elf)
    local initrd_args=() payload=
    [[ "$profile" != release ]] || cargo_args+=(--release)
    case "$scenario" in
        isolation) cargo_args+=(--features user-isolation-probe) ;;
        write-rx) cargo_args+=(--features user-write-probe) ;;
        execute-nx) cargo_args+=(--features user-execute-probe) ;;
        deadlock) cargo_args+=(--features cooperative-deadlock-probe) ;;
        elf|payload-missing|payload-invalid|payload-exit)
            cargo_args+=(--features userspace-elf-probe)
            case "$scenario" in
                elf) payload=$("$repo_dir/scripts/build-user-elf.sh" riscv64gc-unknown-none-elf "$profile") ;;
                payload-invalid) payload="$trial_dir/invalid.elf"; printf 'invalid ELF' > "$payload" ;;
                payload-exit)
                    payload="$trial_dir/exit.elf"
                    "$trial_dir/payload-fixture" riscv64gc-unknown-none-elf "$payload" ;;
            esac
            [[ -z "$payload" ]] || initrd_args=(-initrd "$payload")
            ;;
    esac
    (cd "$repo_dir/ferber" && cargo build --quiet "${cargo_args[@]}")
    local kernel="$repo_dir/ferber/target/riscv64gc-unknown-none-elf/$profile/ferber"
    if [[ "$scenario" == elf || "$scenario" == payload-* ]]; then
        local hash
        hash=$(sha256sum "$kernel" | cut -d ' ' -f 1)
        if [[ -n "${payload_kernel_hash[$profile]:-}" && "${payload_kernel_hash[$profile]}" != "$hash" ]]; then
            echo 'payload swap rebuilt or changed the kernel' >&2
            exit 1
        fi
        payload_kernel_hash[$profile]=$hash
    fi
    local status=0
    timeout "${limit}s" qemu-system-riscv64 \
        -machine virt -smp "$harts" -display none -bios default \
        -serial "file:$log" -kernel "$kernel" -m "${size}M" "${initrd_args[@]}" \
        >"$qemu_log" 2>&1 || status=$?
    if [[ "$status" -ne 124 ]] \
        || ! grep -Eq '\[mem\]   RAM: 0x[0-9a-f]+\.\.0x[0-9a-f]+, [1-9][0-9]* free pages' "$log" \
        || ! grep -Fq '[mem]   physical allocator: ok' "$log" \
        || ! smoke_log_ok "$log" "$scenario"; then
        cat "$log" "$qemu_log" >&2
        echo "RISC-V smoke failed: $profile $scenario, ${size} MiB, $harts harts (status $status)" >&2
        exit 1
    fi
    echo "RISC-V smoke passed: $profile $scenario, ${size} MiB, $harts harts"
}

for profile in debug release; do
    run_case "$profile" normal 128 1
    run_case "$profile" normal 256 4
    run_case "$profile" isolation 128 1
    run_case "$profile" write-rx 128 1
    run_case "$profile" execute-nx 128 1
    run_case "$profile" elf 128 1
done
run_case debug deadlock 128 1
run_case debug elf 256 4
run_case debug payload-missing 128 1
run_case debug payload-invalid 128 1
run_case debug payload-exit 128 1
smoke_passed=1
