# Ferrum init launches required services from an immutable archive

Ferrum init is an independently compiled integer-only `no_std` Rust ELF on
RISC-V64 and x86-64. It bootstraps the read-only initramfs server, reads a bounded
service configuration through channel IPC, constructs each required service and
retains resident control channels. General service dependencies, restart policy,
a writable filesystem, a global mount namespace, a console and a shell remain
future work.

## Startup and bootstrap version independently

The 32-byte [startup ABI v1](userspace-startup.md) record at initial SP discovers
one bootstrap channel. Its slot is never hardcoded. A separate 32-byte bootstrap
v3 START message uses magic `FRINIT03`, `u16` version `3`, opcode START (`1`),
reserved-zero fields and exact archive length. Exactly two capabilities arrive
in order: a weak authority for the dedicated services Job, then an owned sealed
archive VMO with READ, MAP, INSPECT and TRANSFER rights. Capacity is page-rounded;
the record identifies exact archive bytes. The services Job limit is currently
16 MiB and must accommodate every required child's charged resources.

Init maps and validates the [archive](initramfs-format.md), locates `sbin/initfs`,
and uses the shared bounded ELF parser and task syscalls to construct that child.
Each segment has its own VMO; file bytes are copied, BSS/tails remain zero,
executable backing is sealed before RX publication, and mutable data is RW/NX.
The child receives a guarded stack and one initial thread. It becomes runnable
only after explicit start; closing its controller afterward does not kill it.

Init explicitly installs the initfs session endpoints, starts initfs, unmaps the
archive source and moves its VMO through the initfs control channel. Initfs
accepts the source, maps it RO and closes the received handle. Its mapping
retains backing. Init uses its separate filesystem client channel to read
`etc/services.conf`, validates the entire configuration, and loads required
service executables by canonical archive path. Services receive independent
control channels and only the filesystem access explicitly requested in their
configuration. No parent mappings or handles are inherited.

## Readiness requires every launched process to execute

Init waits for initfs and every configured service to announce READY. It then
sends bootstrap READY (`2`) with the count of required processes, including
initfs, and no attached handles. The kernel accepts counts 1–9 and checks that
same count against independently observed child READY sends. Zero is permitted
by the wire codec for negative fixtures but rejected by the kernel session.

The kernel issues PING challenge 74. Init forwards it to every child control
endpoint and validates matching PONGs before replying to the kernel. Kernel
observation additionally ties each READY and matching PONG to a distinct live
process directly admitted to the services Job and that process's endpoint.
Init's PING must use the paired endpoint; a reply cannot reuse evidence from
before the kernel issued its challenge. Endpoint observations are weak and
cannot retain closed channels.

Service/control messages are 16 little-endian bytes: magic `FRSVC001`, version
`1`, opcode and challenge. READY (`1`) and SHUTDOWN (`4`) use zero challenge;
PING (`2`) and PONG (`3`) preserve their challenge. Initfs separately serves
bounded [filesystem requests](initramfs.md) on delegated client channels.

## Rooted waiting remains a legitimate steady state

Init waits on its kernel control channel and child control channels. Initfs
waits on its control channel and active filesystem sessions. The bounded
multi-channel wait interface permits either protocol to make progress without
polling an arbitrary endpoint. Closure, cancellation and readiness revalidate
stored wait identity; no interrupt or preemptive scheduler is implied.

The kernel registers its bootstrap peer as a terminal source with weak endpoint
identity and a checked generation. The scheduler's wait graph connects blocked
receivers to potential producers through live WRITE capabilities. Every blocked
component must reach a registered source. A closed channel cycle or an unrelated
blocked task cannot become legitimate merely because init itself is waiting.
The final idle check also requires every recorded service to remain alive and
blocked on the expected control endpoint. The kernel halts with interrupts
masked only after that validated quiescent state.

The separate `service-wake-probe` feature queues challenge 75 after the first
valid idle state and retains a software source lease until all required PONGs
return. This proves cooperative software delivery and resumption, not IRQ or
SMP safety.

## Finite cleanup precedes the resident session

A finite shutdown run warms admission and queue capacities; a repeated measured
run must restore physical frames, capability nodes and root process count.
SHUTDOWN reaches every configured service and initfs. Children exit zero; init
closes its endpoints and exits with expected status one. Dispatcher reclamation
retires the tasks before the launch Job is killed and detached. The final
resident run deliberately keeps those processes alive.

Normal init boots require `memory_verification`, `boot_payload`, `service_launch`,
`multiservice_idle`, `init_ready` and `init_waiting`, each with `[FERBER_TEST]` and
PASS. The wake build additionally requires `software_wake: PASS`. Those markers
establish the checked protocol and waiting contracts, not a complete operating
system. Missing/malformed sources, startup failure, unexpected child exit and
unrooted waiting are failures; a timeout alone is never evidence of success.

Host tests validate archive/tools, filesystem wire/session behavior, service
configuration, bootstrap records, ELF parsing and cleanup accounting. Architecture
boots establish actual execution and message exchange separately from host tests.

Historical physical evidence applies to the v0.3.0 init/service implementation:
a 32 GiB MSI B650M-A PRO WIFI boot reached its six normal markers with 31.46 GiB
managed RAM and the pre-release `0.2.14` banner. The archive/initfs extension
requires new hardware validation. No physical RISC-V result is claimed.

The initramfs extension subsequently passed physical x86-64 validation on the same
32 GiB machine with the pre-bump `0.3.0` banner: all six normal markers passed,
including memory verification, boot payload, service launch, multi-service idle,
init readiness and waiting. The run managed 32218 MiB (31.46 GiB) across 24 extents.
This evidence belongs to that implementation build, not a physically tested
`0.3.1` ISO. The version-before-hardware workflow was adopted after this run.
