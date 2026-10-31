# Build and boot Ferber

Ferber currently runs verification programs and resident Ferrum services. A
successful boot ends with PASS markers and an idle CPU. It has no interactive
shell yet.

Run commands below from the repository root. The build scripts require Bash,
Python 3, Rust nightly, a linker supplied by the Rust toolchain and ELF tools
such as `readelf`.

## Install the Rust targets

```sh
rustup toolchain install nightly
rustup target add riscv64gc-unknown-none-elf riscv64imac-unknown-none-elf x86_64-unknown-none x86_64-unknown-uefi --toolchain nightly
export RUSTUP_TOOLCHAIN=nightly
```

The kernel's RISC-V build uses the `gc` target; independently compiled integer-only
userspace uses the `imac` target. x86-64 has distinct kernel and UEFI loader targets.

Install `qemu-system-riscv64` for the reference RISC-V machine. x86-64 tests need
`qemu-system-x86_64` and OVMF firmware. ISO creation also needs `xorriso`,
`dosfstools` and `mtools`. Smoke scripts use `timeout` and `rg` (ripgrep).
Distribution package names and OVMF installation paths vary.

## Boot Ferrum on RISC-V

```sh
make run INIT=1
```

This builds init, initfs, the configured services, their archive and the kernel,
then starts a 128 MiB QEMU `virt` machine through OpenSBI. To stop a `-nographic`
QEMU session, press Ctrl-A followed by X.

Use `make build` for the default kernel alone. `make run` without `INIT=1` runs
resident kernel/user verification probes instead of the Ferrum init path.
`make run USERSPACE_ELF_PROBE=1` runs the separate raw-ELF fixture. Run payload
selectors separately; they are not combinable feature bundles.

## Build or test the x86-64 ISO

```sh
make x86-iso INIT=1
bash scripts/x86-iso-smoke.sh init
```

The first command writes `ferber.iso` with the release-profile kernel, UEFI
loader and init/archive envelope. The second builds and boots a separate
temporary ISO through OVMF and checks its output. Its kernel includes automated
log-capture diagnostics, so it is not byte-identical to the ordinary ISO.

Set `OVMF_CODE` and `OVMF_VARS` for the smoke script if your firmware lives
outside its default locations. Keep the firmware code image read-only and use a
writable copy of the variables image for manual QEMU runs. The boot image is
unsigned; firmware Secure Boot policy must permit it.

Use `make x86-preview` to build and validate the freestanding kernel ELF. See the
[UEFI loader guide](../../boot/uefi/README.md) for FAT staging and handoff details.
Do not run raw-device write commands without identifying the target drive;
writing an ISO to a USB device replaces its contents.

## Recognize a successful init boot

The init path requires these exact serial markers, each with the prefix shown:

```text
[FERBER_TEST] memory_verification: PASS
[FERBER_TEST] boot_payload: PASS
[FERBER_TEST] service_launch: PASS
[FERBER_TEST] multiservice_idle: PASS
[FERBER_TEST] init_ready: PASS
[FERBER_TEST] init_waiting: PASS
```

The machine then remains idle. A timeout alone is not a pass. A panic or missing
marker needs diagnosis even if other checks succeeded. The dedicated software-wake
build additionally requires `software_wake: PASS`; it is a separate test image.

These markers establish the specified test path, not universal hardware support
or production readiness. See [verification](../reference/verification.md) for
what host, emulator and hardware results demonstrate.

## Inspect failures and debug

```sh
make debug INIT=1
```

QEMU starts paused with a GDB server on port 1234. Connect with a debugger that
supports RISC-V, load the kernel symbols, attach to `:1234` and continue.

The broader suites are `bash scripts/riscv-smoke.sh`, `make x86-smoke` and the
scenario-specific ISO smoke command above. They include deliberate protection
faults and diagnosed failure cases. Their expected terminal states differ from
a normal init boot; use their parser results rather than interpreting any panic
as interchangeable with a pass.
