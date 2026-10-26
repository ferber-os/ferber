ENTRY(_start)
PHDRS {
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
}
SECTIONS {
    . = 0x400000;
    .text : { *(.text.entry) *(.text .text.*) } :text
    . = 0x410000;
    .seal_probe : { *(.seal_probe) } :text
    . = ALIGN(4096);
    .rodata : { *(.rodata .rodata.*) } :rodata
    . = ALIGN(4096);
    .data : { *(.data .data.*) *(.sdata .sdata.*) } :data
    .bss (NOLOAD) : { *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) } :data
    /DISCARD/ : { *(.eh_frame*) *(.comment*) *(.note*) }
}
