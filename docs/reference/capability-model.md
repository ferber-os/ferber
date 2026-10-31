# Capabilities and authority

A capability identifies a kernel object and grants a specific set of operations on it.
Programs use local handle numbers to refer to capabilities. A handle in one process has
no meaning in another process's table.

Processes begin with empty handle tables. Bootstrap and process creation install only
explicitly delegated capabilities. This removes implicit resource inheritance; it does
not prevent an authorized service from misusing its own authority. Services must still
validate requests and avoid acting as confused deputies.

## Objects and local handles

Kernel objects include Jobs (process containers and resource budgets), Processes
(address spaces and handle tables), Threads (execution contexts), VMOs (memory backing),
Channels (message endpoints) and Resources (explicit hardware access). An object type
tag is checked together with the actual backing type before use.

A numeric handle cannot create an object reference. A syscall resolves it in the
caller's table, checks liveness and the rights required by the operation. Numbers are
reused after close; retaining a number is not retaining authority.

<a id="rights"></a>

## Rights limit operations

```rust
bitflags! {
    pub struct Rights: u32 {
        const DUPLICATE  = 1 << 0;  // create another handle to the same node
        const TRANSFER   = 1 << 1;  // move handle into another process
        const READ       = 1 << 2;  // read from object
        const WRITE      = 1 << 3;  // write to object
        const EXECUTE    = 1 << 4;  // map VMO as executable
        const MAP        = 1 << 5;  // map VMO into address space
        const SIGNAL     = 1 << 6;  // raise signals on object
        const WAIT       = 1 << 7;  // wait for signals
        const INSPECT    = 1 << 8;  // read metadata without using object
        const MANAGE     = 1 << 9;  // lifecycle operations (kill, destroy)
    }
}
```

INSPECT permits type/rights metadata without granting general use of an object. Rights
are operation-specific: possession alone does not make every operation legal. Installed
mappings and object state, such as VMO sealing, impose additional rules beyond the
bitmask.

## Handle operations and ownership

The syscall boundary exposes close, duplicate, inspect, attenuate, and revoke on both
architectures. Every process starts with an empty table; later delegation supplies its
authority.

Duplicate requires `DUPLICATE` and creates another local handle to the same capability
node with exactly the same rights. It does not allocate a child node. Closing a handle
removes only that table entry and releases its owned node reference, even if the node
was revoked. It does not revoke aliases. Node revocation invalidates every alias and
descendant; revoked handles can still be closed.

Inspect requires `INSPECT` and returns only the object type and rights through a
validated eight-byte user buffer. It does not reveal pointers or dereference the
underlying object. See [the syscall ABI](syscall-abi.md) for the wire format and errors.

The current table holds 256 handles and reuses vacant numeric slots. A closed number can
later name a different capability. Tables, typed handles, and messages own
`Arc<CapNode>` references. Nodes own their parent and object, so descendants retain
their ancestry and backing. Nodes and objects are reclaimed after their final owners
disappear; closing the last numeric handle need not remove every owner. Unshared
ancestry chains are dropped iteratively. Resident thread exit reclaims the retired
thread and, after its last thread exits, its process. Final drops happen outside locks
on the permanent-root dispatcher stack.

The global factory limits live and reserved nodes to 1,024. Roots and derived children
consume quota; duplicate aliases do not. Fallible allocation and checked `u64` IDs
report exhaustion without leaking reserved quota. This is a live-node bound, not a
permanent array or lifetime allocation limit.

## Derivation creates a narrower branch

When passing a handle to another process, you can only pass equal or fewer rights. Never
more. `sys_cap_attenuate` requires a valid source with `DUPLICATE`, then validates the
requested mask and its subset relationship. Equal or zero rights are allowed and produce
a distinct child node; the source stays in its table. Undefined or oversized masks
return `-22`; recognized rights escalation returns `-13`. See the [syscall
ABI](syscall-abi.md) for allocation errors and precedence.

A duplicate names the same node. A derivation creates a new child node. This matters for
revocation: revoking a derived child leaves siblings and ancestors usable; revoking the
shared node invalidates all aliases through it.

## Typed Rust APIs supplement syscall validation

The kernel has a `Handle<T, R>` wrapper whose type and rights markers distinguish Rust
APIs at compile time. `PhantomData` adds no storage for those markers. This helps typed
callers express intended access. Its constructors do not prove that markers match node
rights; it does not replace runtime checks on raw syscall handles, revocation, object
backing or mapping permissions.

## Transfer preserves explicit ownership

Capabilities move across processes with strict linear ownership semantics:

- **`transfer_send(handle)`**: Requires `Rights::TRANSFER`. Removes that handle from the sender's table. Independently duplicated handles remain present.
- **`transfer_recv(node)`**: Moves the owned node reference into the receiver's table. A full table returns the error together with that reference, so the caller retains ownership.
- **Message discard**: Releases owned node references without revoking shared
  nodes. Independently duplicated aliases and descendants remain valid. Only an
  explicit authorized revoke invalidates them.

The resident channel syscalls move up to 64 capabilities per message. Send requires
`WRITE` on the endpoint and `TRANSFER` on every payload capability; nonblocking receive
requires `READ`, while blocking receive requires `READ | WAIT` even when data is ready.
A rejected send preserves every source slot. A rejected receive leaves the message
queued, the receiver table unchanged, and all user output unchanged. Repeated source
slot numbers are invalid, while distinct slots aliasing one node can be moved together.
Queued nodes that become revoked remain closeable but convey no usable authority when
received.

The channel interface accepts only VMO and current leaf Resource payload backing. Both
the declared tag and actual backing must match. Channel and task objects can retain
queues directly or through handle tables, so they are rejected until graph-owning
transfers have a complete lifetime policy. The same restrictions apply to attenuated or
duplicated aliases. A capability's type tag alone never authorizes a cast: dispatch also
checks the backing's runtime type.

Blocking receive retains the original capability identity across suspension. Closing its
source slot cancels the wait with an invalid-handle error, including after a readiness
notification; slot reuse cannot change its authority. Revoking the node or an ancestor
cancels it as revoked. Dispatcher safe points revalidate waits before resumed delivery.
Neither cancellation nor revocation consumes a message or changes user output.
Revocation does not destroy the endpoint; only its final backing-owner release closes
that side of the pair.

## Revocation invalidates a node and descendants

Ferber uses indirect capability nodes. Handles resolve through capability nodes that own object references. `sys_cap_revoke`
requires `MANAGE` and invalidates the selected node with one Release store. Every lookup
checks the node and its ancestors with Acquire loads. Aliases and descendants then
return `CapError::Revoked` (`-116` at the syscall boundary); ancestors and siblings
survive. Repeated revocation returns `-116`, while close remains permitted.

Revocation denies later capability operations. It does not tear down existing VMO
mappings, destroy processes, or replace task cleanup.

```text
process A: handle -> node 7 -> VMO
process B: handle -> node 7 -> VMO

revoke node 7:
  process A: next capability operation fails
  process B: next capability operation fails
```

The revocation operation changes node state; subsequent lookups through either alias
fail. It does not traverse and remove every process-table entry.

Capability lookup walks the immutable parent chain, so its cost grows with derivation
depth. It does not scan the global node factory or traverse children.

## Lineage is not a global namespace

Nodes retain immutable parent references. The kernel creates root authority and explicit
delegation/derivation distributes it. Ancestry records what a capability derives from,
but a public `fer cap audit` command is not implemented yet.

For filesystem access, the delegated session channel is the authority. Pathnames and
session-local file IDs select objects within that session; they do not name arbitrary
kernel objects. See [filesystem sessions](initramfs.md).

## VMO byte syscalls preserve shared backing

[VMO creation](vmo-abi.md) returns fresh zeroed RAM with READ, WRITE, DUPLICATE,
TRANSFER, INSPECT and MANAGE with creation options zero. The explicit mappable option
adds MAP; the executable creation option requires MAP opt-in and adds EXECUTE. RX
requires [sealed backing](vmo-sealing.md). Duplicate and derived capabilities share the
same backing and its byte-operation lock; attenuation limits a particular capability,
not every other alias. Channel transfer retains the backing and its user-allocation
charge through delivery or message discard. Final ownership, rather than creator exit or
a handle count, determines reclamation. Inspection reports type and rights, not whether
backing is mutable.

[User mappings](vmo-mapping.md) retain `Arc<Vmo>` backing, not capability nodes. Closing
or revoking every handle can therefore reclaim nodes while mapped frames remain owned
and charged. Revocation prevents new capability operations but does not remove
previously installed mapped access. Unmap is authorized by the caller's exact process
mapping record and requires no surviving handle. Sealing requires MANAGE and EXECUTE,
affects the entire shared object and is irreversible. Writable mappings anywhere block
sealing; old WRITE rights do not bypass a sealed object's state. Capability inspection
still reports rights, not object mutability.
