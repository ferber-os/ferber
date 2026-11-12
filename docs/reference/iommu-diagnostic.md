# QEMU DMA isolation diagnostic

The x86-64 DMA diagnostic exercises AMD-Vi translation with QEMU's EDU endpoint.
It runs before ordinary kernel startup and does not launch Ferrum, publish a
Device capability or enable interrupt routing. It is an emulator-only test image.

```bash
QEMU_X86=$(bash scripts/build-qemu-dma.sh)
export QEMU_X86
bash scripts/iommu-smoke.sh
```

The smoke suite builds separate release-profile images for successful transfers,
unmapped access, write-protected backing, access after unmap and production-policy
rejection. A sixth boot adds an unexpected endpoint and requires topology rejection.
`OVMF_CODE` and `OVMF_VARS` override the firmware paths.

## The topology is part of the test contract

The harness uses one CPU, Q35, AMD-Vi, one EDU endpoint at `00:04.0`, and an
explicit VGA function at `00:02.0`. The VGA function provides the framebuffer
required by the UEFI loader; the diagnostic grants it no DMA access. No default
network or USB devices are present. The firmware boots from the diagnostic ISO. The
kernel requires the exact seven-function PCI inventory before activation and rejects a
changed or ambiguous topology, unsupported translation extensions and shared
requester aliases. Root-bus structural exceptions belong to this explicitly
bounded emulator contract; they do not authorize physical IOMMU takeover.

QEMU does not advertise the SMI-filter feature required by Ferber's production
activation policy. Only the build-gated diagnostic entry point permits its absence.
The production policy continues to reject that configuration. The diagnostic keeps
device interrupts masked and uses HPET-bounded polling. Its result establishes no
SMI or interrupt isolation guarantee.

## The emulator's fault address must be trustworthy

The reproducible test emulator is QEMU 11.1.3 with a one-line AMD-Vi fault-encoder
backport. `scripts/build-qemu-dma.sh` downloads the release archive, verifies its
SHA-256 and requires the expected source line before applying the correction. It
builds only the x86 system emulator under ignored `target/qemu-dma/11.1.3` and does
not replace the system QEMU. The script requires a C compiler, Ninja,
Python with venv, setuptools and wheel support, pkg-config, GLib development
headers and libfdt. The archive bundles the required Meson wheel for offline configuration.

The release's event encoder creates a 64-bit mask from the absolute field start
rather than its position within the current 64-bit word. For an event address
starting at bit 64, that can encode the wrong address. The backport changes
`MAKE_64BIT_MASK(start, length)` to `MAKE_64BIT_MASK(bitpos, length)`. Ferber continues
to require an exact fault-address match; it does not accept an incorrect address
to accommodate the emulator. Unpatched QEMU can therefore fail the negative tests.

The harness uses `QEMU_X86` for the selected executable and explicitly enables
`dma-remap` when the model exposes that option. Older emulator versions are not
validated merely because they accept the same device identity.

Deny-default DTE encoding and publication order are checked by host tests. The
initial unassigned DTE is not required to emit an IO_PAGE_FAULT: the guest negative
tests instead exercise an assigned domain's missing or restricted translations.
QEMU's mode-zero path bypasses read/write checks, so these results do not establish
that every architectural AMD-Vi mode is faithfully emulated.

## A completed transfer and a denied transfer require different evidence

The successful scenario copies a known pattern through explicitly mapped IOVAs in
both directions and verifies the destination and neighboring guard bytes. The
negative DMA scenarios require an IOMMU fault matching the tested requester and
address as well as unchanged protected memory. An expired deadline, unrelated
fault or missing fault is a failure. Each deliberate fault runs in a separate boot
because faults poison the assignment. The post-unmap case first performs and
verifies a transfer through that exact mapping, warming the hardware translation
cache before removal.

Each boot must produce exactly one scenario-specific marker:

```text
[FERBER_TEST] qemu_dma_allowed: PASS
[FERBER_TEST] qemu_dma_unmapped: PASS
[FERBER_TEST] qemu_dma_readonly: PASS
[FERBER_TEST] qemu_dma_unmap: PASS
[FERBER_TEST] qemu_dma_production: PASS
[FERBER_TEST] qemu_dma_topology: PASS
```

The last two markers mean that the expected configuration was rejected before
activation. They do not mean that a transfer succeeded. The harness rejects panic,
FAIL markers, duplicate results and results from the wrong scenario. After printing
its result, the diagnostic halts until QEMU is stopped.

## Physical takeover remains a separate boundary

The existing [xHCI handoff diagnostic](xhci-handoff.md) leaves the selected physical
controller stopped. It does not activate AMD-Vi. Successful QEMU DMA isolation does
not establish physical firmware-DMA draining, peer-to-peer containment, requester
coverage or SMI isolation. Clearing bus mastering and observing Transactions
Pending clear do not, by themselves, drain preexisting posted writes.

Local verification passed all six diagnostic boots with the pinned emulator,
51 RISC-V and 44 x86-64 architecture smoke cases, the release-profile x86 ISO boot
and all four xHCI handoff cases. Host tests include 23 engine cases, ten eligibility
cases and a production-owner retention test covering five allocation/activation
outcomes. An isolated test mutation replacing DMA invalidation with interrupt
invalidation made the warmed post-unmap scenario fail with `missing DMA fault`.
These results are emulator and host evidence, not physical isolation verification.

The diagnostic reuses owned VMOs and AMD-Vi invalidation/completion machinery.
Tables and potentially reachable backing remain retained until reboot after
uncertain activation or teardown. No normal boot path gains a DMA grant from these
tests. USB command rings, IRQ delivery and userspace delegation remain separate
work.

The device protocol is described in QEMU's [EDU specification](https://www.qemu.org/docs/master/specs/edu.html).
The fault encoder can be compared in the [release source](https://github.com/qemu/qemu/blob/v11.1.3/hw/i386/amd_iommu.c)
and [upstream implementation](https://github.com/qemu/qemu/blob/master/hw/i386/amd_iommu.c).
