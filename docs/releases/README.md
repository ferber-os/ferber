# Release history

Each note describes Ferber at that release. Older releases used the Wolfram name, different artifacts and earlier interfaces. Commands, limitations and validation evidence are historical; use the current guides and references for the current system.

| Release | Change |
| --- | --- |
| [v0.3.1](Uranium-238-v0.3.1.md) | Ferrum now reads named boot files through a userspace filesystem service and starts multiple configured services. |
| [v0.3.0](Uranium-238-v0.3.0.md) | Ferrum init now constructs and launches a separately built resident service. |
| [v0.2.14](Uranium-238-v0.2.14.md) | Physical allocation charges now follow their original Job budget through transfer, creator exit and final destruction. |
| [v0.2.13](Uranium-238-v0.2.13.md) | Userspace can seal VMO backing and execute it through RX mappings. |
| [v0.2.12](Uranium-238-v0.2.12.md) | Userspace can now map and unmap owned VMOs. |
| [v0.2.11](Uranium-238-v0.2.11.md) | Userspace gains owned-RAM VMO creation and byte reads/writes. |
| [v0.2.10](Uranium-238-v0.2.10.md) | Ferrum init now starts as an independently delivered Rust executable and remains alive while waiting for messages. |
| [v0.2.9](Uranium-238-v0.2.9.md) | The boot environment now supplies the userspace ELF independently of the kernel binary. |
| [v0.2.8](Uranium-238-v0.2.8.md) | A versioned startup record lets independently compiled Rust code discover its delegated channel without assuming a handle number. |
| [v0.2.7](Uranium-238-v0.2.7.md) | The ELF fixture now receives one explicitly delegated bootstrap channel after its initially empty handle table is created. |
| [v0.2.6](Uranium-238-v0.2.6.md) | A separately compiled static ELF now executes in an isolated process on both architectures. |
| [v0.2.5](Uranium-238-v0.2.5.md) | Cooperative userspace tasks can now yield, block for channel messages and exit independently. |
| [v0.2.4](Uranium-238-v0.2.4.md) | Userspace gains nonblocking channel creation, messaging and capability transfer. |
| [v0.2.3](Uranium-238-v0.2.3.md) | Wolfram replaces the old physical-RAM ceiling with sparse firmware-map discovery and permanent kernel mappings. |
| [v0.2.2](Uranium-238-v0.2.2.md) | Userspace gains capability attenuation and subtree revocation. |
| [v0.2.1](Uranium-238-v0.2.1.md) | Userspace gains handle close, duplicate and inspect operations on both architectures. |
| [v0.2.0](Uranium-238-v0.2.0.md) | User probes now cross into U-mode or Ring 3, return through kernel syscalls and exercise memory isolation on both architectures. |
| [v0.1.7](Uranium-238-v0.1.7.md) | FastCall adds synchronous register exchange and thread donation to the kernel IPC primitives. |
| [v0.1.6](Uranium-238-v0.1.6.md) | Paired channels now carry bounded byte messages and capability references. |
| [v0.1.5](Uranium-238-v0.1.5.md) | Jobs, processes and threads now form an explicit lifetime hierarchy. |
| [v0.1.4](Uranium-238-v0.1.4.md) | Virtual address spaces now use a shared paging interface on RISC-V 64 and x86-64. |
| [v0.1.3](Uranium-238-v0.1.3.md) | Heap allocations now have an explicit 128 MiB ceiling. |
| [v0.1.2](Uranium-238-v0.1.2.md) | The kernel gains a dynamic buddy heap backed by physical frames. |
| [v0.1.1](Uranium-238-v0.1.1.md) | Wolfram gains the capability types and allocation machinery needed by later kernel objects. |
| [v0.1.0](Uranium-238-v0.1.0.md) | Wolfram reaches its first repeatable boot milestone on RISC-V 64 and x86-64. |
