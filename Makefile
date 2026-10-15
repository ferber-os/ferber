KERNEL := wolfram/target/riscv64gc-unknown-none-elf/debug/wolfram

.PHONY: all build run debug x86-preview x86-smoke x86-iso clean

all: build

build:
	cd wolfram && cargo build

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
	scripts/x86-preview.sh --release
	cd boot/uefi && cargo +nightly build --release --target x86_64-unknown-uefi
	bash scripts/build-iso.sh wolfram/target/x86_64-unknown-none/release/wolfram boot/uefi/target/x86_64-unknown-uefi/release/wolfram-uefi-loader.efi wolfram.iso

clean:
	cd wolfram && cargo clean
