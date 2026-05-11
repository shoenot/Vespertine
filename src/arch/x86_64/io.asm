section .text

global outb
outb:
    mov dx, di
    mov al, sil
    out dx, al
    ret

global inb
inb:
    mov dx, di
    xor rax, rax
    in al, dx
    ret
    
global outl
outl:
    mov dx, di
    mov eax, esi
    out dx, eax
    ret

global inl
inl:
    mov dx, di
    xor rax, rax
    in eax, dx
    ret
