#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
target=${1:?target required}; profile=${2:?profile required}
init=${4:-$("$repo_root/scripts/build-user.sh" init "$target" "$profile")}
service=$("$repo_root/scripts/build-user.sh" service "$target" "$profile")
initfs=$("$repo_root/scripts/build-user.sh" initfs "$target" "$profile")
output=${3:-"$repo_root/target/userspace-init/$target/$profile/boot.bundle"}
mkdir -p "$(dirname "$output")"
root=$(mktemp -d /tmp/ferber-initramfs-root.XXXXXX)
trap 'rm -rf -- "$root"' EXIT
mkdir -p "$root/sbin" "$root/etc" "$root/libexec/services" "$root/share/empty"
cp "$initfs" "$root/sbin/initfs"
cp "${5:-$service}" "$root/libexec/services/alpha"
cp "$service" "$root/libexec/services/beta"
printf 'version 1\nservice alpha libexec/services/alpha fs\nservice beta libexec/services/beta none\n' > "$root/etc/services.conf"
printf 'Ferber initramfs\n' > "$root/share/readme"
archive="$(dirname "$output")/initramfs"
"$repo_root/scripts/initramfs.sh" pack "$root" "$archive" >/dev/null
python3 - "$init" "$archive" "$output" <<'PYTHON'
import struct,sys
from pathlib import Path
parts=[Path(p).read_bytes() for p in sys.argv[1:3]]
content=bytearray(96); entries=[]
for kind,data in zip((1,3),parts):
 content.extend(bytes((-len(content))%8)); offset=len(content); content.extend(data)
 entries.append(struct.pack('<IIQQQ',kind,0,offset,len(data),0))
if len(content)>4*1024*1024: raise SystemExit('boot bundle exceeds 4 MiB')
content[:32]=struct.pack('<8sIIQQ',b'FRBOOT01',2,2,len(content),0)
content[32:96]=b''.join(entries)
Path(sys.argv[3]).write_bytes(content)
PYTHON
realpath "$output"
