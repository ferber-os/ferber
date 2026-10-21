KERNEL := ferber/target/riscv64gc-unknown-none-elf/debug/ferber
USERSPACE_ELF_PROBE ?= 0

ifeq ($(USERSPACE_ELF_PROBE),1)
KERNEL_FEATURES = --features userspace-elf-probe
RISCV_PAYLOAD = -initrd target/userspace-elf/riscv64gc-unknown-none-elf/debug/probe.elf
X86_PAYLOAD = target/userspace-elf/x86_64-unknown-none/release/probe.elf
X86_ISO_BUILD = scripts/x86-preview.sh --release --features userspace-elf-probe
else
X86_ISO_BUILD = scripts/x86-preview.sh --release
endif

.PHONY: all build riscv-payload run debug x86-preview x86-smoke x86-iso clean

all: build

build:
	cd ferber && cargo build $(KERNEL_FEATURES)

riscv-payload:
ifeq ($(USERSPACE_ELF_PROBE),1)
	scripts/build-user-elf.sh riscv64gc-unknown-none-elf debug >/dev/null
endif

run: build riscv-payload
	qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-bios default \
		-kernel $(KERNEL) \
		-m 128M $(RISCV_PAYLOAD)

debug: build riscv-payload
	qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-bios default \
		-kernel $(KERNEL) \
		-m 128M $(RISCV_PAYLOAD) \
		-s -S

x86-preview:
	scripts/x86-preview.sh

x86-smoke:
	scripts/x86-smoke.sh

x86-iso:
ifeq ($(USERSPACE_ELF_PROBE),1)
	scripts/build-user-elf.sh x86_64-unknown-none release >/dev/null
endif
	$(X86_ISO_BUILD)
	cd boot/uefi && cargo +nightly build --release --target x86_64-unknown-uefi
	bash scripts/build-iso.sh ferber/target/x86_64-unknown-none/release/ferber boot/uefi/target/x86_64-unknown-uefi/release/ferber-uefi-loader.efi ferber.iso "$(X86_PAYLOAD)"

clean:
	cd ferber && cargo clean
