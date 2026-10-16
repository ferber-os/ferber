//! Host tests for boot-map parsing and heap-free physical allocation.
#![allow(dead_code)]

#[path = "../wolfram/src/kernel/memory/bitmap.rs"]
mod bitmap;
#[path = "../wolfram/src/boot_info.rs"]
mod boot_info;
#[path = "../wolfram/src/arch/riscv64/device_tree.rs"]
mod device_tree;
#[path = "../wolfram/src/kernel/memory/phys_map.rs"]
mod phys_map;
