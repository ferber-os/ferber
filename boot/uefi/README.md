# Boot Ferber through UEFI

The x86-64 UEFI loader reads `FERBER.ELF` and an optional `BOOT.ELF` from its FAT
filesystem, prepares the kernel handoff and exits firmware boot services. Init
builds use `BOOT.ELF` to carry the init/archive envelope; the name does not mean
that every payload is a raw ELF file.

## Build the normal init image

From the repository root, with Rust nightly, both x86 targets and ISO tools
installed:

```sh
make x86-iso INIT=1
bash scripts/x86-iso-smoke.sh init
```

The first command creates `ferber.iso`. The smoke command builds and boots a
separate temporary ISO through QEMU/OVMF with automated logging enabled. Set
`OVMF_CODE` and `OVMF_VARS` if your firmware paths differ. See
[Running Ferber](../../docs/guides/running.md) for setup and expected output.

## Stage a FAT filesystem directly

To build the pieces separately:

```sh
rustup target add x86_64-unknown-none x86_64-unknown-uefi --toolchain nightly
scripts/x86-preview.sh --release
(cd boot/uefi && cargo +nightly build --target x86_64-unknown-uefi --release)
boot/uefi/package.sh KERNEL_ELF FAT_ROOT LOADER_EFI BOOT_ELF
```

Replace the uppercase arguments with actual paths. The third argument is the
loader EFI binary; the optional fourth is the payload. Packaging writes:

```text
EFI/BOOT/BOOTX64.EFI
FERBER.ELF
BOOT.ELF                # only when a payload is supplied
```

The two-argument form uses the default release loader and omits the payload.
Omitting a payload removes a stale staged `BOOT.ELF`. The loader reads from its
own FAT filesystem, so placing files only in the ISO's outer filesystem is not
sufficient. ISO packaging embeds a FAT El Torito boot image.

The loader is unsigned. Secure Boot policy must allow it. It requires a GOP
framebuffer mode using 32-bit RGB or BGR pixels. GOP is the firmware framebuffer
interface; a general GPU driver is not required for boot diagnostics.

## The kernel ELF has a restricted contract

The loader accepts x86-64 ELF64 `ET_DYN`, with no dynamic interpreter or shared
library dependencies and only `R_X86_64_RELATIVE` relocations. The preview script
checks that freestanding contract before boot. The entry uses the System V ABI
and receives a `BootInfo *` in `RDI`.

A contiguous physical allocation holds the loaded image. Relocations write into
that allocation and produce addresses relative to the high virtual image base
`0xffff_ffff_8000_0000`. The loader jumps to the high entry after exiting boot
services; the kernel selects its image-backed high stack before Rust execution.

## Firmware memory remains reserved after handoff

BootInfo version 4 is the shared 168-byte structure carrying the memory map,
framebuffer and optional external payload. Physical reservation addresses and
readable virtual pointers are distinct. Only `EfiConventionalMemory` is eligible
RAM at this stage. Loader code, data, runtime/firmware allocations and boot data
remain reserved; reclaiming them requires a separate ownership transition.

A present payload must be nonempty and at most 4 MiB. The loader reads its exact
length into dedicated zeroed pages and closes file handles before the final
memory-map/ExitBootServices sequence. Only file-not-found means absent. Other
read failures stop handoff. The resident probe path permits absence; ELF and
init builds require their corresponding payload. See the
[boot-payload reference](../../docs/reference/boot-payload.md).

Temporary four-level tables cover the kernel, boot data, framebuffer and the
identity/scratch mappings needed during transition. Kernel segments receive
explicit writable/executable permissions. The kernel later installs permanent
supervisor mappings and packed RAM aliases, removes transition windows and
validates physical-address width and alias capacity. Newly created user roots
share upper-half kernel mappings and start with empty lower halves.

## Validation has separate emulator and hardware scope

`make x86-smoke` exercises normal and deliberate fault paths. ISO smoke exercises
the CD-ROM boot path using the shared packager. Neither result alone proves
physical compatibility. Physical x86-64 evidence, its exact build provenance and
limitations are recorded in the [verification reference](../../docs/reference/verification.md)
and [release history](../../docs/releases/README.md).
