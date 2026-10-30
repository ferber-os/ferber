#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-initramfs-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
rustc --edition=2021 -D warnings --test "$repo_root/shared/fs_tests.rs" -o "$test_dir/filesystem-tests"
"$test_dir/filesystem-tests"
rustc --edition=2021 -D warnings "$repo_root/scripts/initramfs-tool.rs" -o "$test_dir/initramfs"
python3 - "$test_dir/initramfs" <<'PY'
import os, pathlib, struct, subprocess, sys, tempfile, unittest
TOOL=sys.argv.pop()
class Tools(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory(prefix='ferber-initramfs-host.')
  self.base=pathlib.Path(self.temp.name); self.root=self.base/'root'; self.root.mkdir()
  (self.root/'sbin').mkdir(); (self.root/'sbin'/'init').write_bytes(b'init executable bytes')
  (self.root/'etc').mkdir(); (self.root/'etc'/'services').write_bytes(b'one\ntwo\n')
  (self.root/'empty').write_bytes(b''); self.archive=self.base/'initramfs'
 def tearDown(self): self.temp.cleanup()
 def run_tool(self,*args,ok=True):
  p=subprocess.run([TOOL,*map(str,args)],capture_output=True)
  self.assertEqual(p.returncode==0,ok,p.stderr.decode())
  return p
 def pack(self): self.run_tool('pack',self.root,self.archive)
 def test_reproducible_roundtrip(self):
  self.pack(); before=self.archive.read_bytes()
  os.utime(self.root/'sbin'/'init',(123456,123456)); self.pack()
  self.assertEqual(before,self.archive.read_bytes())
  self.run_tool('verify',self.archive)
  listing=self.run_tool('list',self.archive).stdout.decode().splitlines()
  self.assertEqual([x.split('\t')[2] for x in listing],['empty','etc','etc/services','sbin','sbin/init'])
  destination=self.base/'unpacked'; self.run_tool('extract',self.archive,destination)
  self.assertEqual((destination/'sbin'/'init').read_bytes(),(self.root/'sbin'/'init').read_bytes())
  self.assertEqual((destination/'etc'/'services').read_bytes(),b'one\ntwo\n')
  self.assertEqual((destination/'empty').read_bytes(),b'')
 def test_existing_destinations_never_overwritten(self):
  self.pack(); target=self.base/'existing'; target.mkdir(); (target/'sentinel').write_bytes(b'keep')
  self.run_tool('extract',self.archive,target,ok=False)
  symlink=self.base/'link'; symlink.symlink_to(target,target_is_directory=True)
  self.run_tool('extract',self.archive,symlink,ok=False)
  self.assertEqual((target/'sentinel').read_bytes(),b'keep'); self.assertEqual(list(target.iterdir()),[target/'sentinel'])
 def test_source_symlinks_special_files_and_in_tree_output_rejected(self):
  self.archive.write_bytes(b'previous output')
  link=self.root/'link'; link.symlink_to(self.root/'sbin'/'init')
  self.run_tool('pack',self.root,self.archive,ok=False); self.assertEqual(self.archive.read_bytes(),b'previous output'); link.unlink()
  os.mkfifo(self.root/'pipe'); self.run_tool('pack',self.root,self.archive,ok=False); (self.root/'pipe').unlink()
  self.run_tool('pack',self.root,self.root/'archive',ok=False)
  symlink=self.base/'root-link'; symlink.symlink_to(self.root,target_is_directory=True)
  self.run_tool('pack',symlink,self.archive,ok=False)
 def test_invalid_archive_rejected_before_extraction(self):
  self.pack(); data=bytearray(self.archive.read_bytes())
  # Mutate the last record: a valid earlier init cannot hide malformed tail metadata.
  count=struct.unpack_from('<I',data,12)[0]
  struct.pack_into('<I',data,32+(count-1)*32+12,1)
  self.archive.write_bytes(data); destination=self.base/'invalid-dest'
  for command in ['verify','list']: self.run_tool(command,self.archive,ok=False)
  self.run_tool('extract',self.archive,destination,ok=False); self.assertFalse(destination.exists())
 def test_entry_and_size_limits(self):
  for i in range(65): (self.root/f'f{i:02}').write_bytes(b'')
  self.run_tool('pack',self.root,self.archive,ok=False)
  for p in self.root.glob('f*'): p.unlink()
  (self.root/'huge').write_bytes(bytes(4*1024*1024+1))
  self.run_tool('pack',self.root,self.archive,ok=False)
 def test_path_escape_and_names_rejected(self):
  self.pack(); data=bytearray(self.archive.read_bytes()); count=struct.unpack_from('<I',data,12)[0]
  data[32+count*32]=ord('/')
  self.archive.write_bytes(data); destination=self.base/'escape'
  self.run_tool('extract',self.archive,destination,ok=False); self.assertFalse(destination.exists())
  (self.root/'bad name').write_bytes(b'')
  self.run_tool('pack',self.root,self.base/'another',ok=False)
unittest.main()
PY
