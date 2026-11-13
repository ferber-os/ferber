# Read-only IOMMU readiness diagnostic

This x86-64 diagnostic inventories PCI functions and their firmware-described AMD-Vi
coverage before considering physical activation. It reports what is visible and
what remains unproved. It does not activate the IOMMU, reset controllers, clear bus
mastering, mask device interrupts or delegate a Device capability.

## Build the profile matching the selected controller

For QEMU:

```bash
bash scripts/build-iommu-readiness.sh qemu
bash scripts/iommu-readiness-smoke.sh
```

The smoke suite boots the inventory image with one `qemu-xhci` controller and AMD-Vi.
The tested QEMU AMD-Vi model exposes an overlapping dummy MSI capability in its
conventional chain. The suite requires an unknown observation for that exact
function and an inventory FAIL, followed by readiness BLOCKED. It does not apply
the separate DMA diagnostic's model-specific capability exception.
Separate boots require explicit rejection of missing IVRS data, a missing controller
and duplicate target controllers. `OVMF_CODE` and `OVMF_VARS` select the firmware
files, using the same conventions as other UEFI smoke suites.

For the dedicated AMD `1022:15b8` controller on the MSI B650M-A PRO WIFI:

```bash
bash scripts/build-iommu-readiness.sh msi
```

This produces `ferber-iommu-readiness-msi.iso`. The loader captures the selected
controller's resource metadata using the existing handoff record. The kernel then
runs the inventory diagnostic instead of the controller handoff or ordinary startup.
The captured resource is evidence about identity and bounds, not permission to
activate DMA isolation.

## A complete inventory can still be blocked

A successfully collected snapshot ends with:

```text
[FERBER_TEST] iommu_inventory: PASS
[FERBER_TEST] iommu_readiness: BLOCKED
```

`iommu_inventory: PASS` means the diagnostic collected its required inventory.
`BLOCKED` is the expected activation result. It does not claim requester quiescence,
firmware-DMA draining, peer-to-peer containment or interrupt isolation. A malformed,
missing or unreadable prerequisite instead produces an inventory FAIL with its
stage. There is no readiness or isolation PASS marker.

The diagnostic validates the conventional 256-byte snapshot before deciding whether
extended capabilities apply. Host bridges, PCI Express functions and PCI-X functions
advertising 266/533 MHz support are extended-space candidates. Other conventional
functions are labeled `conventional-only`; the probe does not read beyond `0xff`
for them. Invalid conventional metadata remains unknown. This classification is
limited to this read-only report and changes no activation eligibility checks.

The MSI board's SMBus `0000:00:14.0` and LPC `0000:00:14.3` functions advertise no
conventional capability list. They therefore use the conventional-only path. The
host bridge `0000:00:00.0` remains an extended-space candidate: its observed
`0x14d81022` at `0x100` is still an unresolved malformed snapshot. A repeated device
ID is not accepted as proof that extended capabilities are absent.

An extended-capability decode failure is reported as `UNKNOWN`, with the function's
full PCI address, raw header at `0x100` and last-read context. Unknown entries are
repeated together after the final counters, using cached original reads rather
than reading PCI configuration again. The final summary starts on a fresh framebuffer
page; QEMU debug-port output is unchanged. Very small displays or many unknown entries can
still exceed one page. The diagnostic
continues collecting the other functions, then prints its summary and inventory
FAIL if any extension snapshot remains unknown. `N/A` labels conventional-only functions. `-` means a successfully parsed
snapshot did not contain that capability; `UNKNOWN` does not mean absent or disabled.
Other collection failures can still stop the report early. The final counters
separate conventional-only functions, functions with known enabled extensions and
functions with unknown state.


The report separates requester IDs and aliases, IOMMU coverage and routing facts
from requirements that the current kernel cannot establish. Observing bus mastering
disabled or Transactions Pending clear cannot by itself prove that earlier posted
DMA writes have drained. Firmware tables and PCI configuration readbacks also do
not establish a complete peer-to-peer containment contract.

The diagnostic halts after its report and does not continue into Ferrum. Record the
source commit, image SHA-256 and actual output when reporting a hardware result.
No physical activation or DMA grant follows from a successful inventory.

## This complements two different diagnostics

The [xHCI handoff diagnostic](xhci-handoff.md) actively stops and resets the selected
controller. The [QEMU DMA diagnostic](iommu-diagnostic.md) exercises translation and
denial in a bounded emulator contract. The readiness image performs neither action;
it gathers the evidence needed to identify the remaining physical takeover work.

## Configuration accesses follow the processor contract

PCI configuration MMIO uses explicit accumulator-register MOV instructions, as
required by AMD PPR 56713 revision 3.05, section 2.1.5.1 (pages 41–42). The kernel
preserves alignment, range and UC mapping checks. Ordinary device BAR and IOMMU
MMIO retain their existing accessors. The UEFI loader reads configuration through
the firmware PCI I/O protocol rather than direct ECAM access.

Host tests check access widths and bounds, and inspect debug and optimized code
generation for EAX/AX accesses. These checks establish the instruction contract;
they do not explain the root complex's repeated device ID or establish DMA isolation.
