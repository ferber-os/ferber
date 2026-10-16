    .section .text.boot
    .globl _start
_start:
    la t0, boot_hart_claimed
    li t1, 1
    amoswap.w.aq t2, t1, (t0)
    bnez t2, secondary_hart_park

    # Firmware enters without paging. The low boot island owns its tables;
    # runtime Rust starts only after both PC and SP have high addresses.
    la t0, boot_root
    la t1, boot_low
    srli t1, t1, 2
    ori t1, t1, 1
    sd t1, 16(t0)
    la t0, boot_low
    li t1, 0x200800cf
    sd t1, 8(t0)

    la t0, boot_high
    la t1, runtime_physical_literal
    ld t1, 0(t1)
    srli t1, t1, 2
    ori t1, t1, 0xcf
    li t2, 512
2:
    sd t1, 0(t0)
    li t3, 0x80000
    add t1, t1, t3
    addi t0, t0, 8
    addi t2, t2, -1
    bnez t2, 2b
    la t0, boot_root
    la t1, boot_high
    srli t1, t1, 2
    ori t1, t1, 1
    li t2, 4080
    add t0, t0, t2
    sd t1, 0(t0)

    # A bounded two-GiB FDT window also works for sparse physical memory.
    mv a2, a1
    srli t1, a1, 30
    slli t1, t1, 28
    ori t1, t1, 0xc3
    la t0, boot_root
    li t2, 4032
    add t0, t0, t2
    sd t1, 0(t0)
    li t2, 0x10000000
    add t1, t1, t2
    sd t1, 8(t0)
    li t0, 0x3fffffff
    and a1, a1, t0
    li t0, 0xfffffffe00000000
    or a1, a1, t0

    la t0, boot_root
    srli t0, t0, 12
    li t1, 8
    slli t1, t1, 60
    or t0, t0, t1
    csrw satp, t0
    sfence.vma zero, zero
    la t0, runtime_entry_literal
    ld t0, 0(t0)
    jr t0
secondary_hart_park:
    wfi
    j secondary_hart_park

    .align 3
runtime_entry_literal:
    .dword riscv_runtime_entry
runtime_physical_literal:
    .dword __runtime_physical_start

    .section .bss.boot
    .align 12
boot_root:
    .space 4096
boot_high:
    .space 4096
boot_low:
    .space 4096
boot_hart_claimed:
    .word 0

    .section .text.entry
riscv_runtime_entry:
    la sp, boot_stack_top
    call kernel_main
3:
    wfi
    j 3b

    .section .bss.stack
    .align 4
    .globl boot_stack_bottom
boot_stack_bottom:
    .space 4096 * 16
    .globl boot_stack_top
boot_stack_top:

    .section .rodata
    .align 3
    .globl riscv_boot_root_physical
riscv_boot_root_physical:
    .dword boot_root
    .globl riscv_runtime_physical_base
riscv_runtime_physical_base:
    .dword __runtime_physical_start
    .globl riscv_kernel_physical_bounds
riscv_kernel_physical_bounds:
    .dword __kernel_physical_start
    .dword __kernel_physical_end
