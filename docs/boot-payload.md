# External boot payload

The opt-in `userspace-elf-probe` path loads one independently built raw static
ELF supplied separately from the kernel. It uses the existing ELF loader,
startup ABI v1 and bootstrap-channel test protocol. It is not an archive,
initramfs, verified-boot mechanism or service-launching init.

## BootInfo v3 describes exact bytes and reserved backing

BootInfo v3 is 136 bytes. Its appended 24-byte `BootPayload` descriptor contains
three fixed-width `u64` fields in order: `physical_start`, `byte_len`, and
`virtual_start`. `shared/boot_payload.rs` defines the shared descriptor and limits.
All fields zero means absent; partially populated descriptors are invalid.
A present file must have 1 byte through 4 MiB. Checked arithmetic computes
page-rounded backing independently of the exact length; page padding is not
part of the ELF file.

`virtual_start` must equal `0xffff_fffe_0100_0000` plus the physical page offset.
An 8 MiB window is reserved at that base. Both ports validate range, supported
physical width and mapping bounds before exposing a slice. The permanent alias
is supervisor-only RO/NX and remains shared by process roots. No ordinary
writable RAM alias covers its reserved backing.

## RISC-V obtains bounds from the device tree

QEMU delivers the file through `-initrd`. `/chosen/linux,initrd-start` and
`/chosen/linux,initrd-end` are paired physical bounds, with end exclusive.
Each property must be exactly four or eight big-endian bytes; these scalar
widths do not depend on the root memory-node cell counts. Ambiguous chosen
nodes, duplicate properties, missing halves, reversed/empty bounds, oversized
files and unsupported addresses stop boot.

The complete page-rounded backing must lie in reported RAM. It cannot overlap
the kernel/boot island, FDT or any `/reserved-memory` range,
including `no-map`. An overlapping firmware reservation-map entry is accepted
only when its page-rounded extent exactly matches the payload; other overlaps
are rejected. This conservative policy can reject otherwise usable firmware
layouts. It does not treat a reservation as proof of readable RAM.
Like the physical allocator, this path trusts firmware RAM descriptions; it
does not independently audit every device node for overlapping MMIO.

Discovery reserves the source before allocator metadata or bootstrap tables
can claim it. All FDT reads finish before temporary mappings are retired; the
source is consumed through its permanent alias after root replacement.

## UEFI reads a separate file from its own filesystem

The loader optionally opens `\BOOT.ELF` beside `\FERBER.ELF` on its FAT
filesystem. File-not-found means absent. Invalid length and other file or read
errors fail the handoff. It allocates dedicated `EFI_LOADER_DATA` pages, zeros
the rounded allocation and reads the exact bounded file bytes. File handles
close and allocations finish before the final memory-map/ExitBootServices
sequence. The existing map-key retry remains necessary.

UEFI ownership is established from the loader allocation and firmware map,
not merely from an entry marked reserved in the common map. The loader maps
the explicit alias and the kernel recreates it in its permanent root.

## The source survives until shutdown

Payload backing is excluded before general allocation and ordinary RAM-alias
construction. A retained physical VMO owns the source description without
zeroing or freeing firmware pages. Publication waits until validation,
permanent mapping and retained backing are complete.

This is an intentional boot-lifetime reservation. It is outside managed frame
accounting, and there is no payload reclaim operation. Loading copies bytes
into fresh segment VMOs; process teardown reclaims those copies and its stacks,
not the source. Repeated fixture runs must restore their ordinary allocator,
capability, process and scheduler baselines while the source remains pinned.

## Delivery is independent of kernel compilation

```sh
make run USERSPACE_ELF_PROBE=1
make debug USERSPACE_ELF_PROBE=1
make x86-iso USERSPACE_ELF_PROBE=1
```

RISC-V commands build the user fixture separately and pass `-initrd`. The x86
ISO puts `BOOT.ELF` inside its FAT boot image. The directory ESP packager accepts
an optional fourth `BOOT_ELF` argument after `LOADER_EFI`. Building the kernel
alone requires no user executable and embeds no ELF bytes. There is no
`FERBER_USER_ELF` fallback.

Resident configurations allow a missing payload. The ELF configuration requires
one and fails before task admission if absent or invalid. An optional source in
a resident boot is not evidence of its execution.

The ELF scenario requires `memory_verification: PASS`, `userspace_elf: PASS`,
`userspace_bootstrap_channel: PASS`, and `boot_payload: PASS`, each prefixed
`[FERBER_TEST]`. The payload marker follows actual ELF execution, the validated
reply and reclamation checks; discovery or mapping alone cannot satisfy it.
These are verification requirements, not a claim that every configuration or
physical machine has been tested. See [ELF loading](elf-loading.md) for accepted
image restrictions and [startup ABI v1](userspace-startup.md) for the unchanged
entry contract.
