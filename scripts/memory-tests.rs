//! Host tests for boot-map parsing and heap-free physical allocation.
#![allow(dead_code)]

#[path = "../ferber/src/kernel/memory/bitmap.rs"]
mod bitmap;
#[path = "../ferber/src/boot_info.rs"]
mod boot_info;
#[path = "../ferber/src/arch/riscv64/device_tree.rs"]
mod device_tree;
#[path = "../ferber/src/kernel/memory/phys_map.rs"]
mod phys_map;
