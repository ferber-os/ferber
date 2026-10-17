    .section .text.entry, "ax"
    .global _start
    .type _start, @function

_start:
    # The loader supplies a stack and BootInfo pointer in RDI (SysV ABI).
    # UEFI boot services have already ended; keep interrupts disabled.
    cli
    cld
    lea rsp, [rip + boot_stack_top]
    and rsp, -16
    call kernel_main

1:
    hlt
    jmp 1b

    .section .bss.boot_stack, "aw", @nobits
    .balign 4096
boot_stack:
    .skip 65536
boot_stack_top:
