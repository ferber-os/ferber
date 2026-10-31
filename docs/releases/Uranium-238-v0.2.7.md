# Uranium-238 v0.2.7

*The ELF fixture now receives one explicitly delegated bootstrap channel after its initially empty handle table is created. A finite receive/reply/close/exit exchange verifies authority handoff and cleanup.*

## The test fixture proves delegation follows empty-at-birth

The process is constructed with an empty handle table. Checkpoint B then
installs one bootstrap channel endpoint in slot 0 with `READ | WRITE` rights
before scheduler admission. The kernel prequeues a challenge. Independently
linked RISC-V and x86-64 fixtures receive it through the existing nonblocking
channel receive syscall, send a reply through the existing nonblocking send
syscall, close the delegated handle, and exit. The kernel checks the exact
reply and confirms task, handle, and scheduler cleanup before emitting
`[FERBER_TEST] userspace_bootstrap_channel: PASS`.

No syscall numbers or production service tasks are added. Slot 0 and this
challenge/reply exchange belong only to the test ABI. Init startup, argument
and environment setup, and a production bootstrap service protocol remain
future work.

## Checkpoint A remains the same bounded static loader

ELF64 `ET_EXEC` validation, VMO-backed load segments, strict page permissions,
zeroed BSS, guarded 16 KiB user stack, scheduler admission, and repeated
reclamation remain in place. Checkpoint A continues to leave the handle table
empty. Checkpoint B uses the same validation and loader, then delegates the
endpoint explicitly before the task can run.

## Local validation covers both fixtures and the release image

All 41 host tests pass: 15 ELF parser, five loader, nine memory, and 12
channel tests. All 27 QEMU/ISO boots pass: 13 RISC-V cases, 12 x86 UEFI cases,
and both normal and ELF release-ISO boots. The ELF ISO smoke checks the actual
release packaging path. Kernel and probe Clippy checks pass with `-D warnings`
for both targets; release UEFI Clippy and x86 ELF checks pass as well. Hosted
GitHub Actions is not claimed as locally run.

The ELF build booted on physical x86-64 hardware. The
`userspace_elf` and `userspace_bootstrap_channel` markers passed. No physical
RISC-V run is claimed. The boot preceded the version metadata bump.

The kernel and UEFI loader advance to `0.2.7` without dependency upgrades.
The release workflow publishes the opt-in ELF checkpoint ISO as `ferber.iso`.
It remains a prerelease. Version `0.3.0` is reserved for the next milestone.
