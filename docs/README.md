# Ferber documentation

Ferber is a capability-based microkernel; Ferrum supplies its userspace services.
The current system boots init, serves immutable boot files and starts configured
programs. An interactive shell, general service manager and persistent storage
are still planned.

## Start with the question you have

| You want to… | Read |
| --- | --- |
| Build or boot the system | [Running Ferber](guides/running.md) |
| Understand the kernel and authority model | [Architecture](guides/architecture.md) |
| Follow startup from firmware to services | [Boot and services](guides/boot-and-services.md) |
| Understand a term | [Glossary](reference/glossary.md) |
| Inspect demonstrated behavior and checks | [Verification](reference/verification.md) |
| Contribute a change | [Contribution guide](../CONTRIBUTING.md) |
| See future priorities | [Roadmap](../ROADMAP.md) |
| Understand a particular release | [Release history](releases/README.md) |

## Exact contracts

The references define the current implementation. Their bounds and failure
rules matter when constructing programs or reviewing kernel changes.

- Authority and communication: [capabilities](reference/capability-model.md),
  [IPC](reference/ipc.md), [syscalls](reference/syscall-abi.md).
- Program startup: [ELF loading](reference/elf-loading.md),
  [startup record](reference/userspace-startup.md),
  [task construction](reference/task-creation.md), [init](reference/init.md).
- Memory: [physical and virtual memory](reference/memory.md),
  [Job accounting](reference/job-accounting.md), [VMO operations](reference/vmo-abi.md),
  [mappings](reference/vmo-mapping.md), [executable sealing](reference/vmo-sealing.md).
- Boot files: [payload delivery](reference/boot-payload.md),
  [boot envelope](reference/boot-bundle.md), [archive format](reference/initramfs-format.md),
  [filesystem sessions](reference/initramfs.md).

RISC-V64 and x86-64 have matching shared interfaces. QEMU coverage exists for both;
physical validation is x86-64 only. Release notes preserve the scope of each
historical result. A design constraint or test pass is not a complete security
proof.
