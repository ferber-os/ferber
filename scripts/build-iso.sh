#!/usr/bin/env bash
# Package the release kernel and UEFI loader as an El Torito bootable ISO.
set -euo pipefail

if [[ "$#" -ne 3 ]]; then
    echo "usage: $0 KERNEL_ELF LOADER_EFI OUTPUT_ISO" >&2
    exit 2
fi
kernel=$1 loader=$2 output=$3
for required in "$kernel" "$loader"; do
    if [[ ! -f "$required" ]]; then
        echo "missing ISO input: $required" >&2
        exit 1
    fi
done
for command in xorriso mkfs.vfat mmd mcopy; do
    if ! command -v "$command" >/dev/null; then
        echo "missing command: $command" >&2
        exit 1
    fi
done

# Size FAT32 image to payload + 10 MiB headroom (FAT32 minimum: 33 MiB).
payload_kb=$(du -sk "$kernel" "$loader" | awk '{s+=$1} END {print s}')
img_kb=$(( (payload_kb + 10240 + 511) / 512 * 512 ))
if [[ "$img_kb" -lt 33792 ]]; then
    img_kb=33792
fi

iso_work=$(mktemp -d)
trap 'rm -rf -- "$iso_work"' EXIT

efi_img="$iso_work/efiboot.img"
truncate -s "${img_kb}k" "$efi_img"
mkfs.vfat -F 32 -n WOLFRAM "$efi_img" >/dev/null
mmd -i "$efi_img" ::/EFI ::/EFI/BOOT
mcopy -i "$efi_img" "$loader" ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$efi_img" "$kernel" ::/WOLFRAM.ELF

mkdir -p "$iso_work/iso/EFI/BOOT"
cp "$efi_img" "$iso_work/iso/EFI/BOOT/efiboot.img"
xorriso -as mkisofs -R -J -V WOLFRAM \
    -e EFI/BOOT/efiboot.img -no-emul-boot \
    -o "$output" "$iso_work/iso"

echo "ISO: $(du -sh "$output" | cut -f1)"
