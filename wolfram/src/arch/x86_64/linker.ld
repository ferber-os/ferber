ENTRY(_start)

SECTIONS {
    . = 0;
    __kernel_start = .;

    .text : ALIGN(4096) {
        __text_start = .;
        *(.text.entry)
        *(.text .text.*)
    }
    . = ALIGN(4096);
    __text_end = .;
    .rodata : ALIGN(4096) {
        __rodata_start = .;
        *(.rodata .rodata.*)
    }
    . = ALIGN(4096);
    __rodata_end = .;
    .data : ALIGN(4096) {
        *(.data .data.*)
    }
    .bss : ALIGN(4096) {
        *(.bss .bss.*)
        *(COMMON)
    }

    . = ALIGN(4096);
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame)
    }
}
