#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 4 ]; then
    echo "usage: $0 KERNEL_ELF OUTPUT_DIR [LOADER_EFI] [BOOT_ELF]" >&2
    exit 2
fi

script_dir=$(cd -- "$(dirname -- "$0")" && pwd)
kernel_elf=$1
output_dir=$2
loader_efi=${3:-"$script_dir/target/x86_64-unknown-uefi/release/ferber-uefi-loader.efi"}

if [ ! -f "$kernel_elf" ]; then
    echo "missing kernel ELF: $kernel_elf" >&2
    exit 1
fi
if [ ! -f "$loader_efi" ]; then
    echo "missing UEFI loader: $loader_efi" >&2
    exit 1
fi

if [ -n "${4:-}" ] && [ ! -f "$4" ]; then
    echo "missing boot ELF: $4" >&2
    exit 1
fi

install -Dm0644 -- "$loader_efi" "$output_dir/EFI/BOOT/BOOTX64.EFI"
install -Dm0644 -- "$kernel_elf" "$output_dir/FERBER.ELF"

if [ -n "${4:-}" ]; then
    install -Dm0644 -- "$4" "$output_dir/BOOT.ELF"
else
    rm -f -- "$output_dir/BOOT.ELF"
fi
echo "Staged boot files in $output_dir"
