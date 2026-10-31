# Ferrum init and required services

Init is Ferrum’s first userspace process. It starts the read-only filesystem service,
reads `etc/services.conf`, prepares the listed executables and starts each required
service. A failure of a required component fails startup.

The normal example starts `initfs` and two service processes. All remain alive after
readiness and wait on explicitly delegated channels. There is currently no shell,
restart policy, service dependency graph or writable filesystem.

Read the [boot-to-services guide](../guides/boot-and-services.md) first for the
sequence. This page describes the authority, lifecycle and supervision contracts.

## Init loads the filesystem before configured services

Init maps and validates the [archive](initramfs-format.md), locates `sbin/initfs`, and
uses the shared bounded ELF parser and task syscalls to construct that child. Each
segment has its own VMO; file bytes are copied, BSS/tails remain zero, executable
backing is sealed before RX publication, and mutable data is RW/NX. The child receives a
guarded stack and one initial thread. It becomes runnable only after explicit start;
closing its controller afterward does not kill it.

Init explicitly installs the initfs session endpoints, starts initfs, unmaps the archive
source and moves its VMO through the initfs control channel. Initfs accepts the source,
maps it RO and closes the received handle. Its mapping retains backing. Init uses its
separate filesystem client channel to read `etc/services.conf`, validates the entire
configuration, and loads required service executables by canonical archive path.
Services receive independent control channels and only the filesystem access explicitly
requested in their configuration. No parent mappings or handles are inherited.

## Startup and bootstrap version independently

The 32-byte [startup ABI v1](userspace-startup.md) record at initial SP discovers one
bootstrap channel. Its slot is never hardcoded. A separate 32-byte bootstrap v3 START
message uses magic `FRINIT03`, `u16` version `3`, opcode START (`1`), reserved-zero
fields and exact archive length. Exactly two capabilities arrive in order: a weak
authority for the dedicated services Job, then an owned sealed archive VMO with READ,
MAP, INSPECT and TRANSFER rights. Capacity is page-rounded; the record identifies exact
archive bytes. The services Job limit is currently 16 MiB and must accommodate every
required child's charged resources.

## Readiness requires every launched process to execute

Init waits for initfs and every configured service to announce READY. It then sends
bootstrap READY (`2`) with the count of required processes, including initfs, and no
attached handles. The kernel accepts counts 1–9 and checks that same count against
independently observed child READY sends. Zero is permitted by the wire codec for
negative fixtures but rejected by the kernel session.

The kernel issues PING challenge 74. Init forwards it to every child control endpoint
and validates matching PONGs before replying to the kernel. Kernel observation
additionally ties each READY and matching PONG to a distinct live process directly
admitted to the services Job and that process's endpoint. Init's PING must use the
paired endpoint; a reply cannot reuse evidence from before the kernel issued its
challenge. Endpoint observations are weak and cannot retain closed channels.

Service/control messages are 16 little-endian bytes: magic `FRSVC001`, version `1`,
opcode and challenge. READY (`1`) and SHUTDOWN (`4`) use zero challenge; PING (`2`) and
PONG (`3`) preserve their challenge. Initfs separately serves bounded [filesystem
requests](initramfs.md) on delegated client channels.

## Rooted waiting remains a legitimate steady state

Init waits on its kernel control channel and child control channels. Initfs waits on its
control channel and active filesystem sessions. The bounded multi-channel wait interface
permits either protocol to make progress without polling an arbitrary endpoint. Closure,
cancellation and readiness revalidate stored wait identity; no interrupt or preemptive
scheduler is implied.

The kernel registers its bootstrap peer as a terminal source with weak endpoint identity
and a checked generation. The scheduler's wait graph connects blocked receivers to
potential producers through live WRITE capabilities. Every blocked component must reach
a registered source. A closed channel cycle or an unrelated blocked task cannot become
legitimate merely because init itself is waiting. The final idle check also requires
every recorded service to remain alive and blocked on the expected control endpoint. The
kernel halts with interrupts masked only after that validated quiescent state.

The separate `service-wake-probe` feature queues challenge 75 after the first valid idle
state and retains a software source lease until all required PONGs return. This proves
cooperative software delivery and resumption, not IRQ or SMP safety.

## Failure and cleanup

A required-service failure terminates the services Job and fails startup. Configured
executables are prepared before any configured child starts, so a missing or invalid
later image cannot run an earlier configured service first. Initfs must already be
running to supply those image bytes.

Closing a source handle does not free live mappings. The dispatcher reclaims retired
tasks from its own stack after leaving their address spaces and stacks. A finite
shutdown test precedes the resident session and checks every child's exit and
restoration of frame, capability-node and process counts.

See [verification](verification.md) for the shutdown protocol, software-wake probe,
markers and historical physical evidence. Unexpected user faults remain fatal; startup
supervision is not general service recovery.
