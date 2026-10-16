OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS {
    . = 0x80200000;
    __kernel_physical_start = .;
    .boot : { KEEP(*(.text.boot)) *(.data.boot) }
    .boot.bss (NOLOAD) : { *(.bss.boot) }
    . = ALIGN(0x200000);
    __runtime_physical_start = ABSOLUTE(.);

    . = 0xffffffff80000000;
    __kernel_start = ABSOLUTE(.);
    __text_start = .;
    .text : AT(__runtime_physical_start) { *(.text.entry) *(.text .text.*) }
    . = ALIGN(4096);
    __text_end = .;
    __rodata_start = .;
    .rodata : AT(__runtime_physical_start + ADDR(.rodata) - __kernel_start) {
        *(.rodata .rodata.*)
    }
    . = ALIGN(4096);
    __rodata_end = .;
    .data : AT(__runtime_physical_start + ADDR(.data) - __kernel_start) {
        *(.data .data.*) *(.sdata .sdata.*)
    }
    .bss : AT(__runtime_physical_start + ADDR(.bss) - __kernel_start) {
        sbss = .;
        *(.bss .bss.*) *(.sbss .sbss.*)
        ebss = .;
    }
    . = ALIGN(4096);
    __kernel_end = .;
    __kernel_physical_end = __runtime_physical_start + __kernel_end - __kernel_start;
    /DISCARD/ : { *(.eh_frame) }
}
