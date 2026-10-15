OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = 0x80200000;

SECTIONS {
    . = BASE_ADDRESS;
    __kernel_start = .;

    __text_start = .;
    .text : {
        *(.text.entry)
        *(.text .text.*)
    }

    . = ALIGN(4096);
    __text_end = .;
    __rodata_start = .;
    .rodata : {
        *(.rodata .rodata.*)
    }

    . = ALIGN(4096);
    __rodata_end = .;
    .data : {
        *(.data .data.*)
    }

    .bss : {
        sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        ebss = .;
    }

    . = ALIGN(4096);
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame)
    }
}
