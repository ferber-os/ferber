# The Ferber Capability Model

This is the most important document in the repository.
Everything else is implementation. This is the idea.

---

## What A Capability Is

A capability is an unforgeable token that simultaneously names a resource
and authorizes operations on it.

In most systems, naming and authorization are separate:

```
Linux:
  name:          /etc/passwd        (anyone can name this)
  authorization: UID/GID/ACL check  (kernel decides separately)
```

In Ferber:

```
Ferber:
  name + authorization: the handle itself
  possession = permission. no separate check.
```

If a process holds a handle, it has the authorization the handle encodes.
If it doesn't hold a handle, it cannot name the resource at all.
The confused deputy problem doesn't exist here — authority is explicit, not ambient.

---

## Kernel Objects and Handles

Every resource the kernel manages is a kernel object:

```
Process     — address space + handle table
Thread      — execution context within a process
VMO         — Virtual Memory Object (memory region)
Channel     — bidirectional async message pipe
Job         — process container + policy enforcer
Resource    — hardware access (MMIO ranges, interrupt vectors)
```

Every kernel object has handles. A handle is an unforgeable integer
that lives in a process's handle table. The integer is meaningless
outside that process — you cannot take handle `7` from process A
and use it in process B. The kernel validates every syscall against
the calling process's handle table.

---

## Rights

Every handle carries a rights bitmask:

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

`INSPECT` separates "can use this object" from "can see metadata about it."
A process can hold a channel handle without knowing who is on the other end.

## Handle Syscalls

The resident syscall boundary exposes close, duplicate, inspect, attenuate,
and revoke on both architectures. A process starts with an empty table; the
resident probe receives its test capabilities through explicit kernel delegation
afterward.

Duplicate requires `DUPLICATE` and creates another local handle to the same
capability node with exactly the same rights. It does not allocate a child node.
Closing a handle removes only that table entry and releases its owned node
reference, even if the node was revoked. It does not revoke aliases. Node
revocation invalidates every alias and descendant; revoked handles can still
be closed.

Inspect requires `INSPECT` and returns only the object type and rights through
a validated eight-byte user buffer. It does not reveal pointers or dereference
the underlying object. See [the syscall ABI](syscall-abi.md) for the wire format
and errors.

The current table holds 256 handles and reuses vacant numeric slots. A closed
number can later name a different capability. Tables, typed handles, and messages
own `Arc<CapNode>` references. Nodes own their parent and object, so descendants
retain their ancestry and backing. Nodes and objects are reclaimed after their
final owners disappear; closing the last numeric handle need not remove every
owner. Unshared ancestry chains are dropped iteratively. Resident thread exit
reclaims the retired thread and, after its last thread exits, its process.
Final drops happen outside locks on the permanent-root dispatcher stack.

The global factory limits live and reserved nodes to 1,024. Roots and derived
children consume quota; duplicate aliases do not. Fallible allocation and checked
`u64` IDs report exhaustion without leaking reserved quota. This is a live-node
bound, not a permanent array or lifetime allocation limit.

---

## Attenuation

When passing a handle to another process, you can only pass equal or fewer rights.
Never more. `sys_cap_attenuate` requires a valid source with `DUPLICATE`, then
validates the requested mask and its subset relationship. Equal or zero rights
are allowed and produce a distinct child node; the source stays in its table.
Undefined or oversized masks return `-22`; recognized rights escalation returns
`-13`. See the [syscall ABI](syscall-abi.md) for allocation errors and precedence.

```rust
// you hold READ | WRITE | DUPLICATE on a VMO
// you can give a child READ only:
channel.send(Message {
    handles: [vmo.attenuate(Rights::READ)],
})?;

// you cannot do this — kernel rejects it
channel.send(Message {
    handles: [vmo.attenuate(Rights::READ | Rights::EXECUTE)],
    // EXECUTE wasn't in your rights. rejected.
})?;
```

Authority flows in one direction: downward.
A child cannot have more authority than its parent granted.
A parent cannot grant more than it holds.
The root of all authority is the kernel at boot.

---

## Rust Type Enforcement

The capability model is enforced at runtime by the kernel.
It is also enforced at compile time by the Rust type system.

```rust
pub struct Handle<T: KernelObject, R: RightMarker> {
    raw: u32,
    node: Arc<CapNode>,
    _type: PhantomData<T>,
    _rights: PhantomData<R>,
}

// These are different types. The compiler will not let you mix them.
Handle<Vmo, Read>
Handle<Vmo, ReadWrite>
Handle<Channel, ReadWrite>

// This is a compile error. Not a runtime panic. A compile error.
fn write_vmo(h: Handle<Vmo, ReadWrite>, data: &[u8]) { ... }

let ro: Handle<Vmo, Read> = ...;
write_vmo(ro, data);  // ← does not compile
```

This costs nothing at runtime. `PhantomData` is zero-size.
The type information exists only during compilation.
Two layers of enforcement, one of them free.

---

## Capability Transfer & Move Semantics

Capabilities move across processes with strict linear ownership semantics:
- **`transfer_send(handle)`**: Requires `Rights::TRANSFER`. Removes that handle from the sender's table. Independently duplicated handles remain present.
- **`transfer_recv(node)`**: Moves the owned node reference into the receiver's table. A full table returns the error together with that reference, so the caller retains ownership.
- **Message discard**: Releases owned node references without revoking shared
  nodes. Independently duplicated aliases and descendants remain valid. Only an
  explicit authorized revoke invalidates them.

The resident channel syscalls move up to 64 capabilities per message. Send
requires `WRITE` on the endpoint and `TRANSFER` on every payload capability;
nonblocking receive requires `READ`, while blocking receive requires `READ | WAIT`
even when data is ready. A rejected send preserves every source slot. A rejected
receive leaves the message queued, the receiver table unchanged, and all user
output unchanged. Repeated source slot numbers are invalid, while distinct
slots aliasing one node can be moved together. Queued nodes that become revoked
remain closeable but convey no usable authority when received.

This milestone accepts only VMO and current leaf Resource payload backing.
Both the declared tag and actual backing must match. Channel and task objects
can retain queues directly or through handle tables, so they are rejected until
graph-owning transfers have a complete lifetime policy. The same restrictions
apply to attenuated or duplicated aliases. A capability's type tag alone never
authorizes a cast: dispatch also checks the backing's runtime type.

Blocking receive retains the original capability identity across suspension.
Closing its source slot cancels the wait with an invalid-handle error, including
after a readiness notification; slot reuse cannot change its authority.
Revoking the node or an ancestor cancels it as revoked. Dispatcher safe points
revalidate waits before resumed delivery. Neither cancellation nor revocation
consumes a message or changes user output. Revocation does not destroy the
endpoint; only its final backing-owner release closes that side of the pair.

---

## Revocation

The hard problem of capability systems: once you give someone a handle,
how do you take it back?

Ferber uses indirect capability nodes. Processes don't hold handles
to objects directly — they hold handles to capability nodes that own object
references. `sys_cap_revoke` requires `MANAGE` and invalidates the selected node
with one Release store. Every lookup checks the node and its ancestors with
Acquire loads. Aliases and descendants then return `CapError::Revoked`
(`-116` at the syscall boundary); ancestors and siblings survive. Repeated
revocation returns `-116`, while close remains permitted.

Revocation denies later capability operations. It does not tear down existing
VMO mappings, destroy processes, or replace task cleanup.

```
process A holds: Handle → CapNode(7) → VMO(0xdeadbeef)
process B holds: Handle → CapNode(7) → VMO(0xdeadbeef)

kernel revokes CapNode(7):
  process A's handle → CapNode(7) [INVALID] → error
  process B's handle → CapNode(7) [INVALID] → error
```

One kernel operation. Instant. Both handles dead simultaneously.

Capability lookup walks the immutable parent chain, so its cost grows with
derivation depth. It does not scan the global node factory or traverse children.

---

## Capability Derivation Tree

Every handle has a lineage. Every capability derives from a parent capability.
The tree's root is the kernel at boot — it creates the first capabilities
and hands them to init. Everything else derives from that.

`fer cap audit <pid>` walks this tree for a specific process and shows
exactly where each of its handles came from, and what rights were granted
at each step.

A compromised process's entire authority lineage is auditable in one command.
This is not possible on any mainstream operating system today.

---

## What This Means In Practice

```
Linux:     open("/etc/passwd", O_RDONLY)
           kernel checks UID, GID, permission bits, ACLs, SELinux policy
           five separate layers asked "is this allowed"

Ferber:   file_read(passwd_handle)
           kernel checks: does this process hold passwd_handle?
           does passwd_handle have READ rights?
           that's it. two checks. one data structure.
```

The simplicity isn't a compromise. It's a consequence of getting the model right.

## VMO byte syscalls preserve shared backing

[VMO creation](vmo-abi.md) returns fresh zeroed RAM with READ, WRITE, DUPLICATE,
TRANSFER, INSPECT and MANAGE. It grants no MAP or EXECUTE rights. Duplicate and
derived capabilities share the same backing and its byte-operation lock;
attenuation limits a particular capability, not every other alias. Channel
transfer retains the backing and its user-allocation charge through delivery or
message discard. Final ownership, rather than creator exit or a handle count,
determines reclamation. Generic inspection remains unchanged.
