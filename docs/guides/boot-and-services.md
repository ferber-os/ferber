# Ferrum turns boot bytes into running services

The current init path starts a filesystem and two configured test services. It
ends in validated waiting rather than an interactive prompt. The executables
are independently compiled Rust programs, delivered outside the kernel binary.

## The firmware hands off a bounded payload

On x86-64, the UEFI loader reads the kernel and optional `BOOT.ELF` from its FAT
filesystem. On RISC-V, the kernel discovers initrd bounds through the device
tree. In init builds those bytes contain a boot envelope with an initial init
ELF and an independently versioned initramfs archive. The kernel validates the
envelope, reserves its backing and loads only init.

The envelope selects boot roles; archive entries describe files. The kernel has
no reason to know the pathname of a service executable.

## Init receives enough authority to build its children

A startup record identifies init's bootstrap channel without assuming a handle
number. A message supplies the archive length and two grants: services Job
authority and a sealed archive VMO. The VMO permits reading and mapping bytes;
the Job permits constructing children within its allocation budget.

Init temporarily maps the archive to find `sbin/initfs`, prepares that process,
and installs its server-session endpoints explicitly. After start, init unmaps
its source view and transfers the archive VMO to initfs. The filesystem mapping
keeps the bytes alive after its received handle is closed. Initfs receives no
Job or process-controller authority.

## Configuration determines which services receive filesystem access

Init reads `etc/services.conf` through its filesystem session:

```text
version 1
service alpha libexec/services/alpha fs
service beta libexec/services/beta none
```

Both executables must be valid before either configured child starts. The `fs`
field explicitly gives alpha a filesystem session; beta receives none. The
configuration names required services, not restart rules or dependencies.

Filesystem requests stream file bytes through channels. Object IDs identify
open files only within their session. They cannot be used through another
session or substituted for a kernel capability. General mount namespaces and
persistent storage are not implemented.

## Loading bytes does not make them executable

Init validates ELF segments and builds fresh VMOs for child memory. File bytes
are copied; BSS and page tails remain zero. Executable backing is sealed before
RX mappings are installed, while mutable data remains RW/NX. A source archive
never becomes executable merely because it contains ELF bytes.

Each child starts with its explicitly delegated capabilities and one prepared
thread. Draft ownership handles partial failure. Any required startup failure
fails the boot and tears down the services Job.

## Readiness and waiting are checked separately

Initfs and each configured service announce readiness over their control
channels. A challenge then travels through init to every child and back. The
kernel verifies the originating processes and endpoints, rather than accepting
an aggregate claim from init alone.

Afterward, init waits on its control and child channels; initfs waits on control
and active filesystem sessions. Channel wait sets permit these event loops to
block without polling. The scheduler accepts idle only when blocked tasks reach
registered producer sources and all required children remain alive.

The release path halts the CPU with interrupts masked after checking that state.
The software-wake probe separately demonstrates a queued command resuming the
cooperative system. It does not demonstrate hardware interrupts or SMP.

Read [Running Ferber](running.md) for the expected console output, or the exact
[init](../reference/init.md), [task](../reference/task-creation.md),
[filesystem](../reference/initramfs.md) and [startup](../reference/userspace-startup.md)
references for protocol and lifecycle details.
