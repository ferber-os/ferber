#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
target=${1:?target required}; profile=${2:?profile required}
init=${4:-$("$repo_root/scripts/build-user.sh" init "$target" "$profile")}
service=$("$repo_root/scripts/build-user.sh" service "$target" "$profile")
output=${3:-"$repo_root/target/userspace-init/$target/$profile/boot.bundle"}
mkdir -p "$(dirname "$output")"
python3 - "$init" "$service" "$output" <<'PY'
import struct,sys
from pathlib import Path
parts=[Path(p).read_bytes() for p in sys.argv[1:3]]
content=bytearray(96); entries=[]
for kind,data in enumerate(parts,1):
 content.extend(bytes((-len(content))%8)); offset=len(content); content.extend(data)
 entries.append(struct.pack('<IIQQQ',kind,0,offset,len(data),0))
if len(content)>4*1024*1024: raise SystemExit('boot bundle exceeds 4 MiB')
content[:32]=struct.pack('<8sIIQQ',b'FRBOOT01',1,2,len(content),0)
content[32:96]=b''.join(entries)
Path(sys.argv[3]).write_bytes(content)
PY
realpath "$output"
