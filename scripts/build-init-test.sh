#!/usr/bin/env bash
# Build adversarial init executables; never used by release packaging.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
scenario=${1:?usage: build-init-test.sh SCENARIO TARGET PROFILE [OUTPUT]}
shift
case "$scenario" in
    init-malformed-ready|init-exit-before-ready|init-exit-after-ready|init-close-bootstrap|init-wrong-channel) ;;
    *) echo "unsupported init test scenario: $scenario" >&2; exit 2 ;;
esac
target=${1:?usage: build-init-test.sh SCENARIO TARGET PROFILE [OUTPUT]}
profile=${2:?usage: build-init-test.sh SCENARIO TARGET PROFILE [OUTPUT]}
case "$target" in
    riscv64gc-unknown-none-elf|riscv64imac-unknown-none-elf)
        machine='RISC-V'; rust_target=riscv64imac-unknown-none-elf ;;
    x86_64-unknown-none) machine='Advanced Micro Devices X86-64'; rust_target=$target ;;
    *) echo "unsupported userspace ELF target: $target" >&2; exit 2 ;;
esac
case "$profile" in
    debug) opt_level=1 ;;
    release) opt_level=3 ;;
    *) echo "unsupported userspace ELF profile: $profile" >&2; exit 2 ;;
esac
output=${3:-"$repo_root/target/userspace-init-tests/$target/$profile/$scenario.elf"}
mkdir -p "$(dirname "$output")"
output=$(realpath "$output")
rustc +nightly --edition=2021 --crate-name ferber_init_test \
    --target "$rust_target" -D warnings -C panic=abort -C opt-level="$opt_level" \
    -C relocation-model=static -C linker=rust-lld \
    -C link-arg=-T"$repo_root/userspace/common/linker.ld" \
    -C link-arg=--no-relax -C link-arg=--no-dynamic-linker -C link-arg=--build-id=none \
    --cfg "scenario=\"$scenario\"" \
    "$repo_root/userspace/init-test/main.rs" -o "$output"
readelf -h "$output" | grep -Eq 'Class:.*ELF64'
readelf -h "$output" | grep -Eq 'Type:.*EXEC'
readelf -h "$output" | grep -Fq "$machine"
if readelf -l "$output" | grep -Eq 'INTERP|DYNAMIC|TLS' \
    || readelf -d "$output" | grep -Fq '(NEEDED)' \
    || readelf -r "$output" | grep -Eq 'R_(RISCV|X86_64)_'; then
    echo 'userspace ELF unexpectedly requires a runtime loader or relocations' >&2
    exit 1
fi
printf '%s\n' "$output"
