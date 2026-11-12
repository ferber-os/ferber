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
