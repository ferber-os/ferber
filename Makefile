KERNEL := ferber/target/riscv64gc-unknown-none-elf/debug/ferber
USERSPACE_ELF_PROBE ?= 0
MINIMAL_INIT ?= 0
VMO_PROBE ?= 0
VMO_MAP_PROBE ?= 0
VMO_SEAL_PROBE ?= 0

ifeq ($(VMO_SEAL_PROBE),1)
ifneq ($(MINIMAL_INIT)$(USERSPACE_ELF_PROBE)$(VMO_PROBE)$(VMO_MAP_PROBE),0000)
$(error VMO_SEAL_PROBE must run separately from other payloads)
endif
KERNEL_FEATURES = --features userspace-vmo-seal-probe
RISCV_PAYLOAD = -initrd target/userspace-vmo-seal-probe/riscv64gc-unknown-none-elf/debug/probe.elf
X86_PAYLOAD = target/userspace-vmo-seal-probe/x86_64-unknown-none/release/probe.elf
X86_ISO_BUILD = scripts/x86-preview.sh --release --features userspace-vmo-seal-probe
endif

ifeq ($(VMO_MAP_PROBE),1)
ifneq ($(MINIMAL_INIT)$(USERSPACE_ELF_PROBE)$(VMO_PROBE),000)
$(error VMO_MAP_PROBE must run separately from other payloads)
endif
KERNEL_FEATURES = --features userspace-vmo-map-probe
RISCV_PAYLOAD = -initrd target/userspace-vmo-map-probe/riscv64gc-unknown-none-elf/debug/probe.elf
X86_PAYLOAD = target/userspace-vmo-map-probe/x86_64-unknown-none/release/probe.elf
X86_ISO_BUILD = scripts/x86-preview.sh --release --features userspace-vmo-map-probe
endif

ifeq ($(VMO_PROBE),1)
ifneq ($(MINIMAL_INIT)$(USERSPACE_ELF_PROBE),00)
$(error VMO_PROBE, MINIMAL_INIT and USERSPACE_ELF_PROBE are mutually exclusive)
endif
KERNEL_FEATURES = --features userspace-vmo-probe
RISCV_PAYLOAD = -initrd target/userspace-vmo/riscv64gc-unknown-none-elf/debug/probe.elf
X86_PAYLOAD = target/userspace-vmo/x86_64-unknown-none/release/probe.elf
X86_ISO_BUILD = scripts/x86-preview.sh --release --features userspace-vmo-probe
endif

ifeq ($(MINIMAL_INIT)$(USERSPACE_ELF_PROBE),11)
$(error MINIMAL_INIT and USERSPACE_ELF_PROBE are mutually exclusive)
endif

ifeq ($(MINIMAL_INIT),1)
KERNEL_FEATURES = --features minimal-init
RISCV_PAYLOAD = -initrd target/userspace-init/riscv64gc-unknown-none-elf/debug/init.elf
X86_PAYLOAD = target/userspace-init/x86_64-unknown-none/release/init.elf
X86_ISO_BUILD = scripts/x86-preview.sh --release --features minimal-init
endif

ifeq ($(USERSPACE_ELF_PROBE),1)
KERNEL_FEATURES = --features userspace-elf-probe
RISCV_PAYLOAD = -initrd target/userspace-elf/riscv64gc-unknown-none-elf/debug/probe.elf
X86_PAYLOAD = target/userspace-elf/x86_64-unknown-none/release/probe.elf
X86_ISO_BUILD = scripts/x86-preview.sh --release --features userspace-elf-probe
else ifeq ($(MINIMAL_INIT)$(VMO_PROBE)$(VMO_MAP_PROBE)$(VMO_SEAL_PROBE),0000)
X86_ISO_BUILD = scripts/x86-preview.sh --release
endif

.PHONY: all build riscv-payload run debug x86-preview x86-smoke x86-iso clean

all: build

build:
	cd ferber && cargo build $(KERNEL_FEATURES)

riscv-payload:
ifeq ($(VMO_SEAL_PROBE),1)
	scripts/build-user.sh vmo-seal-probe riscv64gc-unknown-none-elf debug >/dev/null
endif
ifeq ($(VMO_MAP_PROBE),1)
	scripts/build-user.sh vmo-map-probe riscv64gc-unknown-none-elf debug >/dev/null
endif
ifeq ($(VMO_PROBE),1)
	scripts/build-user.sh vmo-probe riscv64gc-unknown-none-elf debug >/dev/null
endif
ifeq ($(MINIMAL_INIT),1)
	scripts/build-user.sh init riscv64gc-unknown-none-elf debug >/dev/null
endif
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
ifeq ($(VMO_SEAL_PROBE),1)
	scripts/build-user.sh vmo-seal-probe x86_64-unknown-none release >/dev/null
endif
ifeq ($(VMO_MAP_PROBE),1)
	scripts/build-user.sh vmo-map-probe x86_64-unknown-none release >/dev/null
endif
ifeq ($(VMO_PROBE),1)
	scripts/build-user.sh vmo-probe x86_64-unknown-none release >/dev/null
endif
ifeq ($(MINIMAL_INIT),1)
	scripts/build-user.sh init x86_64-unknown-none release >/dev/null
endif
ifeq ($(USERSPACE_ELF_PROBE),1)
	scripts/build-user-elf.sh x86_64-unknown-none release >/dev/null
endif
	$(X86_ISO_BUILD)
	cd boot/uefi && cargo +nightly build --release --target x86_64-unknown-uefi
	bash scripts/build-iso.sh ferber/target/x86_64-unknown-none/release/ferber boot/uefi/target/x86_64-unknown-uefi/release/ferber-uefi-loader.efi ferber.iso "$(X86_PAYLOAD)"

clean:
	cd ferber && cargo clean
