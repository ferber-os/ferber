#!/usr/bin/env bash
# Build a bounded, independently linked static userspace program.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
program=${1:?usage: build-user.sh PROGRAM TARGET PROFILE [OUTPUT]}
shift
case "$program" in
    init) source_dir=init; output_dir=userspace-init; basename=init ;;
    vmo-probe) source_dir=vmo-probe; output_dir=userspace-vmo; basename=probe ;;
    elf-probe) source_dir=elf-probe; output_dir=userspace-elf; basename=probe ;;
    *) echo "unsupported userspace program: $program" >&2; exit 2 ;;
esac
target=${1:?usage: build-user.sh PROGRAM TARGET PROFILE [OUTPUT]}
profile=${2:?usage: build-user.sh PROGRAM TARGET PROFILE [OUTPUT]}
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
output=${3:-"$repo_root/target/$output_dir/$target/$profile/$basename.elf"}
mkdir -p "$(dirname "$output")"
output=$(realpath "$output")
rustc +nightly --edition=2021 --crate-name "ferber_${basename}" \
    --target "$rust_target" -D warnings -C panic=abort -C opt-level="$opt_level" \
    -C relocation-model=static -C linker=rust-lld \
    -C link-arg=-T"$repo_root/userspace/common/linker.ld" \
    -C link-arg=--no-relax -C link-arg=--no-dynamic-linker -C link-arg=--build-id=none \
    "$repo_root/userspace/$source_dir/main.rs" -o "$output"
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
