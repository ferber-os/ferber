# Ferrum init launches a service and remains alive

Ferrum init is an independently compiled, integer-only `no_std` Rust ELF on
RISC-V64 and x86-64. It receives explicit launch authority and executable source
bytes, constructs a child using task syscalls and retains a resident channel
protocol with that child. It has no filesystem, namespace server, console or
shell, and does not launch arbitrary named services.

## Startup and bootstrap are separate versioned contracts

The 32-byte [startup ABI v1](userspace-startup.md) record at initial SP identifies
one bootstrap channel. Its assigned slot is discovered from the record, never
hardcoded. The kernel gives that endpoint READ, WRITE and WAIT rights.

The kernel prequeues a 32-byte bootstrap v2 START message. Magic `FRINIT02`,
version `2`, opcode START (`1`), reserved-zero fields and exact service ELF byte
length are checked separately from startup. Exactly two attached capabilities
arrive in order: a weak authority for a dedicated 16 MiB services Job, then a
sealed owned source-image VMO with READ, MAP and INSPECT rights. That VMO's rounded
capacity is distinct from the ELF length. No existing parent handles are cloned
into init or its child implicitly.

Init maps the image read-only and validates it with the same bounded ELF parser
used by the kernel. For each segment it creates fresh mappable backing, copies
file bytes through bounded writes and leaves BSS/page tails zero. Executable
backing receives explicit executable creation authority and is sealed before RX
mapping. Mutable data is RW/NX. A draft child receives these mappings, a moved
bootstrap channel endpoint and one initial thread with the standard guarded
stack. Only successful explicit start makes it runnable. Closing the construction
controller afterward does not kill the started child.

Init releases its source mapping, image handle and launch-Job handle after setup.
The structural Job tree and scheduler retain the live service. The exact bundle
format and packaging commands are described in [boot bundles](boot-bundle.md).

## Readiness requires actual service execution

The service validates its startup record and sends READY to init. Init validates
that message, then sends its own bootstrap READY (`2`) with zero service length
and no attached handles. The kernel sends PING challenge 74 to init, which
forwards it to the service. The service returns PONG with the same challenge;
init forwards that reply to the kernel.

The kernel does not accept init's report as evidence that a service ran. It
observes successful channel-send syscalls, checks that the sender is init or a
live process directly admitted to init's service Job, and records the service's
READY and matching PONG from the same endpoint and challenge. Init's PING must
use the paired endpoint. Before reporting terminal idle, the kernel also checks
that this same service process is still alive and blocked receiving on that
endpoint. The observed endpoint is held weakly so verification cannot keep a
closed channel alive.

Service/control messages are 16 little-endian bytes: magic `FRSVC001`, `u16`
version `1`, a `u16` opcode and a `u32` challenge. Opcodes are READY (`1`), PING
(`2`), PONG (`3`) and SHUTDOWN (`4`). READY and SHUTDOWN use challenge zero. The
kernel accepts only exact bootstrap READY/PONG records with no transferred
handles. Init validates service message lengths, fields, opcodes and challenges.

After readiness and request/reply verification, both programs block using normal
READ | WAIT channel receives. No implicit busy loop or kernel-injected runnable
child supplies the success evidence.

## A wait is legitimate only when it reaches an explicit source

The kernel registers its bootstrap peer as a terminal bootstrap source. The
registration holds weak endpoint identity and a checked generation; it does not
keep an endpoint open. Live channel WRITE capabilities in admitted tasks connect
blocked readers to potential producers. The bounded scheduler analysis requires
blocked work to reach a registered source and preserves pending wait-generation,
capability and endpoint validation. Merely holding both sides of a channel does
not turn a closed cycle into an external event source.

Source registration is an internal bootstrap mechanism, not a public syscall or
ambient grant. Cancelling a lease removes source status, while ordinary endpoint
ownership controls actual peer closure. A terminal source marks a permitted
steady state; there is currently no hardware event transport. Once the validated
wait graph is quiescent, the kernel halts with interrupts masked.

The separate `service-wake-probe` feature registers concrete software work,
queues a second PING with challenge 75, resumes the blocked processes and checks
the matching PONG. It releases the software source lease after the response. This
proves software delivery and cooperative wakeup, not IRQ delivery or SMP safety.

## Finite cleanup tests precede the resident boot

The normal initialization path first runs a finite shutdown protocol to warm
admission and queue capacities. A repeated measured run must restore physical
frames, capability nodes and root process count. The kernel sends SHUTDOWN through
init to the service; the service exits zero, and init closes its channels and
exits with expected status one. Both tasks are reclaimed on the dispatcher stack,
then the launch Job is killed and detached after its processes are gone.

The final resident run does not take that exit path. Its required markers are
`memory_verification`, `boot_payload`, `service_launch`, `multiservice_idle`,
`init_ready` and `init_waiting`, each with `[FERBER_TEST]` and PASS. The dedicated
wake build additionally requires `software_wake: PASS` after resumption. Invalid
bootstrap messages, missing payloads, premature init exit, closure and an unrooted
wait remain diagnosed failures; a timeout alone is never success evidence.

Host tests validate bootstrap wire fields, service messages, bundle bounds and
shared ELF parsing. Architecture boot tests establish actual entry, child launch,
message exchange, waiting and the finite reclamation path. Physical x86 evidence
must be recorded separately from QEMU. No physical RISC-V result is implied.

A physical x86-64 boot on the 32 GiB MSI B650M-A PRO WIFI reached all six normal
markers with 31.46 GiB managed RAM. That run used the pre-release `0.2.14`
banner. The dedicated software-wake probe remains QEMU-validated; physical
RISC-V hardware has not been available.
