/* OUTPUT_FORMAT(elf64-x86_64) */

ENTRY(kmain)

PHDRS {
    limine_requests  PT_LOAD FLAGS((1 << 1) | (1 << 2));
    text    PT_LOAD FLAGS((1 << 0) | (1 << 2));
    rodata  PT_LOAD FLAGS((1 << 2));
    data    PT_LOAD FLAGS((1 << 1) | (1 << 2));
}

SECTIONS {
    . = 0xffffffff80000000;

    .limine_requests : {
        KEEP(*(.requests_start));
        KEEP(*(.requests));
        KEEP(*(.requests_end));
    } :limine_requests

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .text : {
        TEXT_BEGIN = .;
        *(.text .text.*)
        TEXT_END = .;
    } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .rodata : {
        RODATA_BEGIN = .;
        *(.rodata .rodata.*)
        RODATA_END = .;
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .data : {
        DATA_BEGIN = .;
        *(.data .data.*)
    } :data

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
        DATA_END = .;
    } :data

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
    }
}
