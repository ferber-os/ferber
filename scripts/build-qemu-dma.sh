#!/usr/bin/env bash
# Build a pinned DMA-test emulator with the upstream AMD-Vi fault-field fix.
set -euo pipefail
[[ $# == 0 ]] || { echo "usage: $0" >&2; exit 2; }
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
version=11.1.3
checksum=5a8289c7940dc77d3873c81a6c29ba3afe1446fc9d36e77fa1658cdbecb84b22
build_root="$repo_root/target/qemu-dma/$version"
archive="$build_root/qemu-$version.tar.xz"
source="$build_root/qemu-$version"
build="$build_root/build"
for command in curl sha256sum tar python3 ninja; do command -v "$command" >/dev/null; done
mkdir -p "$build_root"
if [[ ! -f "$archive" ]]; then
    curl --fail --location --retry 3 "https://download.qemu.org/qemu-$version.tar.xz" -o "$archive.partial" >&2
    mv -- "$archive.partial" "$archive"
fi
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check >&2
if [[ ! -d "$source" ]]; then
    tar -xJf "$archive" -C "$build_root"
fi
# The field start can exceed 63; the mask needs the offset within its uint64_t.
python3 - "$source/hw/i386/amd_iommu.c" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
old = 'uint64_t mask = MAKE_64BIT_MASK(start, length);'
new = 'uint64_t mask = MAKE_64BIT_MASK(bitpos, length);'
if text.count(old) == 1 and text.count(new) == 0:
    path.write_text(text.replace(old, new))
elif text.count(old) != 0 or text.count(new) != 1:
    raise SystemExit('unexpected AMD-Vi fault encoder: refusing to patch')
PY
mkdir -p "$build"
(cd "$build" && "$source/configure" --target-list=x86_64-softmmu \
    --without-default-features --enable-tcg --enable-system --disable-docs \
    --disable-werror --disable-download) >&2
ninja -C "$build" -j "${BUILD_JOBS:-2}" qemu-system-x86_64 >&2
printf '%s\n' "$build/qemu-system-x86_64"
