# Ferber initramfs v1 is a bounded immutable archive

The artifact is named `initramfs`, without an extension. It stores canonical
names and bytes independently of the boot envelope that identifies bootstrap
executables. It carries no permissions, timestamps, ownership, hashes or
implicit resource authority. It is uncompressed and is not an authenticated boot
format or a general filesystem.

## One index describes every byte

`shared/initramfs.rs` is the allocation-free parser shared by the kernel,
userspace and host tools. All integers are little-endian. The 32-byte header is:

| Offset | Bytes | Value |
| --- | --- | --- |
| 0 | 8 | Magic `FRARCH01` |
| 8 | 4 | Version, exactly 1 |
| 12 | 4 | Entry count, at most 64 |
| 16 | 4 | String table byte length |
| 20 | 4 | Absolute data section offset |
| 24 | 4 | Exact total archive byte length, at most 4 MiB |
| 28 | 4 | Reserved, zero |

The index immediately follows the header, with one 32-byte record per entry:

| Offset | Bytes | Value |
| --- | --- | --- |
| 0 | 4 | Path byte offset relative to the string table |
| 4 | 4 | Path byte length, at most 255 |
| 8 | 4 | Kind: 1 regular file, 2 directory |
| 12 | 4 | Reserved, zero |
| 16 | 8 | Absolute file data offset |
| 24 | 8 | Exact file byte length |

Paths are densely concatenated immediately after the index, without NUL
terminators or unused bytes. Records are strictly sorted by pathname bytes;
duplicates are invalid. Each path contains 1–16 components, separated by single
slashes. Components contain only ASCII letters, digits, `.`, `_` and `-`;
`.` and `..` components are invalid. Absolute paths, trailing or repeated
slashes, backslashes, whitespace and non-ASCII names are rejected. Root is
implicit. Every non-root parent must have an earlier directory record.

The data section starts at the 8-byte-aligned end of the strings. Alignment
padding is zero. Directories have zero data offset and length. Each regular
file begins at the next 8-byte-aligned data cursor in record order; padding
between file payloads is zero. Empty files are valid and can share an offset.
The final cursor equals the total archive length, with no trailing bytes.
An empty archive is valid structurally; required bootstrap files are checked
by its consumer.

The parser checks the whole archive before admitting any lookup. Overflow,
truncation, unsupported kinds, nonzero reserved fields, missing/file parents,
ambiguous strings and overlapping or hidden extents fail parsing. `entries()`
returns borrowed metadata and byte slices; `find()` performs binary lookup.
No extraction or heap allocation happens while parsing.

## Host tools use the production parser

The Linux host tools require Rust and use `shared/initramfs.rs` for validation:

```sh
scripts/initramfs.sh pack staging/ target/initramfs
scripts/initramfs.sh list target/initramfs
scripts/initramfs.sh verify target/initramfs
scripts/initramfs.sh extract target/initramfs new-directory
bash scripts/initramfs-tests.sh
```

Packing walks a stable source tree, includes regular files and directories,
rejects symlinks and special files, sorts names, and emits only canonical
metadata and zero padding. The output must be outside the source tree and its
parent directory must already exist. Identical source paths and contents
produce identical archives, independent of host timestamps and modes. Output
is validated, written to an exclusive temporary file and atomically renamed;
packing failure preserves the previous output.

Extraction validates before creating the destination and requires that the
destination not already exist, including as a symlink. It creates a private
root and traverses parents using held directory descriptors with no-follow
opens. Files are created exclusively; existing paths cannot be overwritten.
Only file contents and directory structure are restored, with fixed 0644/0755
creation modes subject to the caller's umask. A failed extraction removes only
the new destination tree. Archive metadata cannot request symlinks, device nodes
or host permissions.
