ENTRY(_start)

SECTIONS {
    . = 0x40000000;

    . = ALIGN(4096);
    .text : {
        *(.text .text.*)
        *(.plt .plt.*)
    }

    . = ALIGN(4096);
    .rodata : {
        *(.rodata .rodata.*)
    }

    . = ALIGN(4096);
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
        *(.got .got.*)
        *(.got.plt .got.plt.*)
    }

    . = ALIGN(4096);
    .bss : {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
    }
}
