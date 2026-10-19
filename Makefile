KERNEL := ferber/target/riscv64gc-unknown-none-elf/debug/ferber
USERSPACE_ELF_PROBE ?= 0

ifeq ($(USERSPACE_ELF_PROBE),1)
X86_ISO_BUILD = FERBER_USER_ELF="$$(scripts/build-user-elf.sh x86_64-unknown-none release)" scripts/x86-preview.sh --release --features userspace-elf-probe
else
X86_ISO_BUILD = scripts/x86-preview.sh --release
endif

.PHONY: all build run debug x86-preview x86-smoke x86-iso clean

all: build

build:
	cd ferber && cargo build

run: build
	qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-bios default \
		-kernel $(KERNEL) \
		-m 128M

debug: build
	qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-bios default \
		-kernel $(KERNEL) \
		-m 128M \
		-s -S

x86-preview:
	scripts/x86-preview.sh

x86-smoke:
	scripts/x86-smoke.sh

x86-iso:
	$(X86_ISO_BUILD)
	cd boot/uefi && cargo +nightly build --release --target x86_64-unknown-uefi
	bash scripts/build-iso.sh ferber/target/x86_64-unknown-none/release/ferber boot/uefi/target/x86_64-unknown-uefi/release/ferber-uefi-loader.efi ferber.iso

clean:
	cd ferber && cargo clean
