global init_clean_fpu_state
global init_xsave
global init_cr4

section .text 

init_xsave:
    ; enable osxsave in cr4
    mov rax, cr4
    or rax, (1 << 18)
    mov cr4, rax

    ; configure xcr0
    xor ecx, ecx  ; ecx = 0 specifies xcr0
    mov eax, 0x7  ; enables x87, SSE, and AVX
    xor edx, edx
    xsetbv        ; writes eax:edx to xcr[ecx]

    ret

init_clean_fpu_state:
    ; rdi = ptr to fpu context buffer
    ; zero out the header first (offset 512 to 576)
    push rdi 
    push rcx
    push rax

    add rdi, 512 
    mov rcx, 8 
    xor rax, rax
    rep stosq

    pop rax 
    pop rcx
    pop rdi

    mov eax, 0x7
    xor edx, edx         ; same as above
    xrstor64 [rdi]
    
    ; init x87
    fninit 
    push 0x1F80
    ldmxcsr [rsp]
    add rsp, 8

    mov eax, 0x7
    xor edx, edx
    xsave64 [rdi]

    ret

init_cr4: 
    mov rax, cr4
    bts rax, 9
    bts rax, 10
    mov cr4, rax
    ret
