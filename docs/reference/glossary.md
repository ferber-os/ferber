# Terms used in Ferber

These definitions describe Ferber's current implementation. The [architecture
guide](../guides/architecture.md) connects them into a system.

| Term | Meaning |
| --- | --- |
| Address space | A process's virtual mappings and private page-table state. Shared supervisor mappings keep the kernel reachable. |
| Alias | Another reference to the same node or another virtual mapping of the same physical bytes; specify which meaning applies. |
| Allocation budget | The accounting owner and quota charged for physical storage and metadata. Ownership survives capability transfer. |
| Backing | The memory or kernel object retained by a VMO, capability node or mapping. |
| Boot envelope | The bounded external file identifying init's ELF and an opaque archive payload. |
| Bootstrap channel | An explicitly installed endpoint discovered through the startup record. It carries later startup messages and grants. |
| Capability | A reference to a kernel object with specific rights and revocation ancestry. |
| Capability node | The shared authority record retained by handles and queued transfers. Derived nodes retain parent nodes. |
| Controller | A Process capability that manages a draft child's construction. It is not the running process itself. |
| Derivation | Creating a child capability node with equal or fewer rights. |
| Dispatcher stack | Kernel-owned stack used after switching away from a task, allowing that task's stacks and address space to be reclaimed. |
| ELF | Executable and Linkable Format; Ferber accepts a bounded static subset for user programs. |
| Ferrum | Ferber's userspace, currently including init, initfs and independently compiled service programs. |
| Handle | A process-local table slot naming a capability node. Numeric slots may be reused. |
| Init | The first Ferrum process, responsible for bootstrapping initfs and constructing configured required services. |
| Initfs | A userspace service that exposes immutable boot-archive files through explicitly delegated sessions. |
| Initramfs | The in-memory immutable archive containing initial userspace files and configuration. |
| Job | An authorizing process container in a hierarchy, with descendant memory accounting and termination. |
| Kernel alias | A supervisor virtual address that maps physical backing for kernel access. It grants no user access. |
| Mapping record | Process-owned metadata retaining VMO backing and tracking an installed mapping's exact extent and permissions. |
| NX | Non-executable memory. RW/NX permits data reads/writes but not instruction fetch. |
| Publication | The point where prepared state becomes visible for use, such as making a task runnable or installing executable mappings. |
| Resident process | A process deliberately kept alive after startup or verification rather than immediately exiting. |
| Revocation | Invalidating a capability node, causing later operations through it or its descendants to fail. Installed mappings have separate lifetime rules. |
| Rooted idle | A blocked task graph whose components remain connected to registered event sources, rather than an unproductive closed wait cycle. |
| Session | A filesystem channel whose possession grants access to its namespace and session-local open-object table. |
| Source lease | A generation-checked registration of a concrete event-producing endpoint. It prevents arbitrary blocked tasks from claiming legitimate idle. |
| VMO | Virtual Memory Object: named backing for memory mappings and byte operations. |
| Wait generation | An identifier distinguishing a current suspended wait from stale notifications belonging to a prior wait. |
| Writable permit | Object-wide ownership retained by a writable mapping; any live permit blocks sealing the entire VMO. |
