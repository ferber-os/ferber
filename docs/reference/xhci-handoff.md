# Selected USB controller handoff diagnostic

The x86-64 handoff image exercises firmware ownership transfer and reset of one
explicitly selected xHCI controller. It leaves that controller halted, with bus
mastering and interrupts disabled. It runs independently of Ferrum init.

This diagnostic does not activate AMD-Vi, delegate a Device capability, start USB
command rings or provide an interactive console. Successful reset does not prove
that all firmware DMA or PCI posted writes have drained.

## Build the controller profile you intend to test

Build the QEMU profile and run its positive and negative boot checks:

```bash
bash scripts/build-xhci-handoff.sh qemu
bash scripts/xhci-handoff-smoke.sh
```

The QEMU profile requires exactly one `qemu-xhci` controller (`1b36:000d`). The
smoke script supplies AMD IOMMU discovery data and rejects missing or duplicate
controllers. A separate negative boot omits IVRS data and requires an explicit
preflight failure. Its firmware paths can be overridden with `OVMF_CODE` and `OVMF_VARS`,
using the same convention as the ordinary ISO smoke script.

For the dedicated controller on the MSI B650M-A PRO WIFI:

```bash
bash scripts/build-xhci-handoff.sh msi
```

This produces `ferber-xhci-msi.iso`. Selection requires a unique AMD `1022:15b8`
xHCI function. The BDF previously observed as `12:00.0` is reconnaissance evidence,
not a hardcoded device address. Use the dedicated controller's ports; shared
controllers carrying the boot USB or unrelated peripherals are outside this probe.
The diagnostic deliberately stops the selected controller until reboot.

## Resource bounds come from firmware before reset

In diagnostic builds, the UEFI loader obtains BAR0 resource metadata through
`EFI_PCI_IO_PROTOCOL.GetBarAttributes` before ExitBootServices. The loader copies
validated values into BootInfo version 5's inline 64-byte controller resource
record. No firmware descriptor pointers survive the handoff. Ordinary loader
builds and RISC-V boots provide an absent record.

The kernel compares that record against live identity and BAR configuration,
validates the aperture and platform descriptions, and checks supported capability
state before the first controller write. It does not size an active BAR or guess
an aperture length from the controller identity. The initial diagnostic accepts
USB Legacy Support (ID 1), Supported Protocol (ID 2) and inactive Debug Capability
(ID 10) extended capabilities.
It bounds each complete record, including protocol speed entries, and rejects
unknown IDs or overlapping register descriptions before writes. Other capability
layouts require explicit support before that controller can pass the probe.
The complete 64-byte Debug Capability must fit without overlap. Its enable and
run bits must be clear during inspection, before takeover and after reset. The
probe never writes Debug Capability registers or takes over an active debug engine.
If the engine becomes active after inspection, the probe fails and attempts PCI
containment.

IVRS coverage remains mandatory for the selected controller and ordinary inventory
functions. The diagnostic permits absent coverage only for a host bridge on the
first bus of a validated MCFG region, or the exact IOMMU function identified by
IVRS with matching IOMMU class. Both require a type-zero PCI header and standard programming interface.
Conflicting coverage and shared requester aliases still reject the probe. These
structural exceptions do not establish global DMA containment or authorize IOMMU
activation.

## Success means the selected controller is stopped

The probe uses HPET deadlines for BIOS ownership release, halt, reset and readiness.
Both 32-bit and 64-bit main counters are supported, with counter-width register
accesses and modular elapsed-time checks. Each deadline must fit within half the
counter range. These are continuously polled boot deadlines: callers must not
leave a deadline unchecked for a complete counter wrap. This is not a persistent
monotonic clock across suspend or arbitrary scheduling delays. At the observed MSI
counter period, a wrap takes about 300 seconds; current hardware waits are at most
one second. A stopped-counter watchdog still fails closed.
It disables defined legacy SMI enables and verifies PCI interrupt masking and bus
master disable. It rechecks identity, BAR placement and final controller state.
Only successful completion emits:

```text
[FERBER_TEST] xhci_handoff: PASS
```

A failure reports its stage and stops the diagnostic. After a partial takeover,
it attempts to contain the selected controller and never resumes ordinary startup.
Unrelated PCI functions are not reset or masked by this probe.

No physical result is implied by the existence of these build commands. A hardware
report should identify the source commit and ISO SHA-256 and include the actual
stage output. Complete requester containment, peer-to-peer isolation, DMA draining
and interrupt routing remain separate prerequisites for userspace assignment.
