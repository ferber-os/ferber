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
only USB Legacy Support (ID 1) and Supported Protocol (ID 2) extended capabilities.
It bounds each complete record, including protocol speed entries, and rejects
unknown IDs or overlapping register descriptions before writes. Other capability
layouts require explicit support before that controller can pass the probe.

## Success means the selected controller is stopped

The probe uses HPET deadlines for BIOS ownership release, halt, reset and readiness.
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
