//! Production ledger/context for harnesses using the host allocator, not the kernel heap.
//! Arena storage is absent here; the dedicated accounting harness exercises real arenas.
#[path = "../ferber/src/kernel/memory/accounting.rs"]
pub mod accounting;
pub mod heap {
    pub struct Arena;
    impl Arena {
        pub const fn new() -> Self { Self }
        pub fn is_empty(&self) -> bool { true }
    }
}
