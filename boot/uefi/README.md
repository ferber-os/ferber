# Wolfram UEFI loader

This standalone Rust `no_std` UEFI application loads `WOLFRAM.ELF` from the
same FAT filesystem as the loader. The kernel must be an x86-64 ELF64
position-independent `ET_DYN` image with only `R_X86_64_RELATIVE` dynamic
relocations and a SysV entry point accepting `BootInfo *` in `RDI`. The loader
asks UEFI for a contiguous physical run of pages and loads the image there.
Relative relocations write to that physical allocation while producing
addresses at the high virtual image base `0xffff_ffff_8000_0000`. After
exiting boot services, the loader enters the high ELF entry; the kernel
selects its high image-backed stack before Rust executes.

Build with the pinned nightly toolchain after installing both x86 targets:

```sh
rustup target add x86_64-unknown-none x86_64-unknown-uefi --toolchain nightly
scripts/x86-preview.sh --release
(cd boot/uefi && cargo +nightly build --target x86_64-unknown-uefi --release)
```

Stage files into a FAT32 partition or a QEMU EFI system partition directory:

```sh
boot/uefi/package.sh /path/to/wolfram-kernel-elf /path/to/fat-root
```

The result contains `EFI/BOOT/BOOTX64.EFI` and `WOLFRAM.ELF`. The image is
unsigned, so firmware Secure Boot policy must permit it. The loader requires a
GOP mode with RGB or BGR 32-bit pixels, and passes its framebuffer and the UEFI
memory map in version 2 of the shared 112-byte `BootInfo` structure. Physical
reservation addresses are distinct from readable virtual pointers, including
the framebuffer's virtual base. Only
`EfiConventionalMemory` is passed as usable. All loader allocations and boot
data are `EfiLoaderCode`/`EfiLoaderData` and remain reserved in the handoff.

Before the jump, the loader installs temporary four-level page tables with
the high kernel, boot-data and framebuffer mappings, scratch pages, and the
identity mappings needed for the transition. Kernel segments receive
per-page writable and executable permissions. The kernel builds packed
aliases for eligible RAM, then replaces this root with permanent supervisor
mappings and removes the identity and scratch windows. New process roots
share the upper half and start with an empty lower half. RAM above the
temporary identity window remains in the handoff and is reached through
scratch-built aliases. The kernel validates supported physical-address
width and alias capacity; it never calls UEFI boot services or reclaims
firmware/loader memory after the handoff.

Debug and release kernels now complete returning userspace syscall probes in
QEMU/OVMF. Separate probes verify supervisor-read, RX-write, and NX-execute
protection faults. Earlier MSI B650M-A PRO WIFI USB boots cover prior
milestones. They do not validate this new memory discovery and high-half
mapping transition; that still needs a fresh hardware boot.

Run `make x86-smoke` from the repository root to repeat the debug and release
userspace roundtrip and protection checks, plus an invalid-opcode exception
probe and debug/release 8 GiB capacity probes. Normal and protection boots
require `memory_verification: PASS` before their resident marker. Capacity
boots also require more than 4 GiB of distinct frames retained together,
then released, and read/write/map/copy/free checks on a frame above 4 GiB.
The smoke script
builds its kernels in a temporary target directory, so its deliberate fault
image cannot replace the ordinary kernel intended for a USB stick.

For an optical-media QEMU test, install `libisoburn`, `dosfstools`, and
`mtools` (Arch package names), then run `make x86-iso`. This creates an ignored
`wolfram.iso` at the repository root. It embeds a FAT El Torito image with both
the UEFI loader and `WOLFRAM.ELF`; placing the ELF only in the ISO filesystem
would not work because the loader reads from its own FAT filesystem. Boot it
with QEMU/OVMF using `-cdrom wolfram.iso -boot order=d`. The ISO tests the CD
boot path; the MSI USB test still uses a FAT32 partition with the two files.

Run `bash scripts/x86-iso-smoke.sh` to build and boot a temporary ISO
automatically. It uses `scripts/build-iso.sh`, the same packager as
`make x86-iso` and the release workflow, and requires the normal userspace
PASS marker. The test kernel adds `qemu-debug-port` diagnostics for log
capture. Set `OVMF_CODE` and `OVMF_VARS` to override the firmware paths.
