global load_gdt

section .text

load_gdt:
    lgdt [rdi]
    push 0x08
    lea rax, [rel .label2]
    push rax
    retfq
    .label2:
    mov ax, 16
    mov ds, ax
    mov es, ax
    mov ss, ax

    mov ax, 0
    mov fs, ax
    mov gs, ax
    
    mov ax, 0x28
    ltr ax

    ret

