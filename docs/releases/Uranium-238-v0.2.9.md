# Uranium-238 v0.2.9

*The userspace Rust ELF now arrives separately from the kernel. RISC-V receives it through the device tree; UEFI reads `BOOT.ELF` beside the kernel. Both execute the existing startup and bootstrap-channel protocol, then verify reclamation. Uranium-238 remains unstable. The executable no longer needs to be embedded to participate.*

## Boot delivery no longer depends on kernel compilation

The opt-in ELF path accepts one raw static ELF of 1 byte through 4 MiB.
BootInfo v3 appends a shared descriptor containing the exact byte length,
physical source address and permanent virtual alias. RISC-V validates paired
four- or eight-byte `/chosen/linux,initrd-start` and `linux,initrd-end`
properties. UEFI optionally reads `\BOOT.ELF` into dedicated, zero-padded
LoaderData pages before its final memory-map handoff.

Kernel builds no longer embed the fixture or require its build-time path.
The Makefile, ESP and ISO packagers, smoke scripts and release workflow build
and deliver the executable separately. Resident builds permit absence; the
ELF path diagnoses missing or invalid payloads before task admission. Startup
ABI v1 and the bootstrap protocol remain unchanged.

## Reserved source pages have a checked supervisor mapping

Discovery reserves the complete page-rounded source before allocator metadata
or bootstrap tables can claim it. The permanent source alias is supervisor
RO/NX, excluded from ordinary writable RAM aliases. Runtime checks validate
each source page before publication. A retained physical VMO describes the
backing without clearing or freeing it. ELF loading copies into fresh owned
segment VMOs; repeated process teardown restores the ordinary accounting
baselines while source pages remain reserved for the boot lifetime.

Bounds, supported address widths, reported RAM coverage and explicit reserved
ranges are checked. Firmware RAM descriptions remain trusted; RISC-V does
not independently audit every device node for overlapping MMIO. This is not
verified boot, an archive format, an initramfs, or service-launching init.

## Adversarial review tightened mapping and failure paths

Three independent QA reviews found and drove fixes for high physical UEFI
addresses clobbering unrelated virtual mappings, payload allocation leaks on
later pre-ExitBootServices failures, and negative-test logs accepting unrelated
failure lines. Focused regressions cover those cases. UEFI cleanup does not
call firmware after an ExitBootServices attempt.

Smoke tests replace the external image with missing, malformed and valid
early-exit payloads while checking that the kernel binary remains unchanged.
Successful execution requires `[FERBER_TEST] boot_payload: PASS` after the
existing ELF protocol and reclamation checks; discovery alone cannot pass.

## Validation covers both ports and physical x86 execution

The full local QEMU matrices passed 17 RISC-V and 15 x86 UEFI cases before
the final QA fixes. Focused post-fix validation passed the RISC-V debug
external-ELF boot at 256 MiB/four harts and the x86 release ELF ISO boot.
The normal release ISO also passed during QA. Host suites passed 17 memory,
eight UEFI payload, 15 ELF parser and seven loader/startup tests. ELF kernel
Clippy checks on both targets and release UEFI Clippy passed with
`-D warnings`; smoke-log self-checks passed.

A physical x86-64 boot on the 32 GiB MSI workstation showed all four markers:
`memory_verification`, `userspace_elf`, `userspace_bootstrap_channel`, and
`boot_payload`. That run used the pre-bump `0.2.8` banner. It demonstrated
execution of the separately delivered ELF and completion of its protocol.
Hosted GitHub Actions results are separate from this local evidence.

The kernel and UEFI loader advance to `0.2.9` without dependency upgrades.
This remains a prerelease; `0.3.0` is reserved. Execution remains cooperative,
integer-only and single-CPU, with interrupts disabled. Init, service launch
and userspace task creation remain pending.
