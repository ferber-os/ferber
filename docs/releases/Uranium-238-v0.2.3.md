# Uranium-238 v0.2.3

*Wolfram replaces the old physical-RAM ceiling with sparse firmware-map discovery and permanent kernel mappings. A physical x86-64 boot validates the implementation with 32 GiB installed.*

## Physical memory discovery follows the firmware map

BootInfo v2 carries validated physical reservations and readable virtual
pointers separately. UEFI marks only conventional memory usable and retains
its memory-map buffer across ExitBootServices retries. RISC-V discovers enabled
FDT memory nodes and static reservations, including `no-map` regions. The
shared map builder merges usable ranges before rounding to full pages, then
subtracts outward-rounded reservations. Sparse holes remain unallocatable.

Allocator metadata is carved from validated RAM and reserved before the
remaining frames are exposed. The bitmap allocator uses page-sized metadata
blocks and tracks sparse extents without a fixed 4 GiB bitmap ceiling. Range
free validates all frames before changing allocator state. Accounting checks
`managed = bootstrap-reserved + allocated + free`.

## Permanent mappings separate RAM, kernel code, and devices

Both architectures transition from bootstrap mappings to permanent high-half
kernel mappings and packed supervisor RAM aliases. RAM aliases are RW/NX;
kernel text is RX, rodata is RO/NX, and data and stacks are RW/NX. RISC-V's
packed alias window is 248 GiB; x86-64 is bounded by its alias window and CPU
physical-address width. Physical holes consume no alias space, while alignment
padding does. Exceeding a window fails boot.

The x86 loader retains its relative-relocation ELF contract and passes physical
reservation addresses separately from virtual boot-data pointers. The
framebuffer uses a dedicated MMIO alias with volatile accesses. Process roots
keep private lower halves and share the permanent supervisor upper half;
address-space teardown frees owned page-table frames without freeing VMO
backing.

Allocated VMO frames are zeroed before exposure, including rounded tails.
Physical VMOs preserve their contents. User copies preflight the entire range
and permissions before writing through supervisor RAM aliases.

## Memory tests now exercise sparse and high frames

Host tests cover BootInfo and FDT validation, extent normalization,
reservation subtraction, and sparse allocator operations. In-kernel checks
cover mapping permissions, alias translations, accounting, VMO clearing and
preservation, user-copy preflight, address-space teardown, and—when the managed
map is at least 4 GiB—an allocatable frame above 4 GiB. The optional
`memory-capacity-probe` retains 1,049,088 distinct frames at once.

On physical x86-64 hardware with 32 GiB installed, the boot reported 8,240,704
managed 4 KiB frames (about 31.4 GiB) across 23 extents. Memory verification,
address-space teardown, user-copy checks, privilege-switch roundtrip, handle
syscalls, and capability mutation passed. The kernel then halted as designed.

The release updates the kernel and UEFI loader to `0.2.3`. Both
kernel targets build and pass Clippy with `-D warnings`; the UEFI loader and all
CI probe configurations do as well. Debug/release x86 ELF checks, all shell
syntax checks, smoke-log parser self-tests, and all nine host memory tests pass.
All 24 QEMU cases pass: 12 RISC-V cases, 11 x86 UEFI cases, and one release
x86 ISO boot. Both architectures pass debug/release 8 GiB capacity probes.
Hosted GitHub Actions has not been run locally.

## Reclaim and concurrent allocation remain future work

UEFI boot-services, runtime, loader, and ACPI memory remain reserved. Reclaim
must wait until loader ownership and all references have ended. The ordinary
high-frame probe is gated by total managed RAM, so a sparse map under 4 GiB
can skip it despite containing a high allocatable frame. The allocator lock is
not interrupt-reentrant; IRQ allocation and SMP use require a reviewed locking
and initialization-publication design. NUMA and DMA policies, hot-add, demand
paging, and scattered kernel stacks are not implemented. A single heap
allocation remains limited to 128 MiB.
