extern syscall_dispatch
global _syscall_entry 
global copy_from_user

section .text

_syscall_entry: 
    swapgs 

    mov [gs:0x08], rsp  ; save user rsp 
    mov rsp, [gs:0x10]  ; load kernel rsp

    push qword [gs:0x08] ; push user rsp on the kernel stack 
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15 
    
    mov rdi, rsp 
    
    call syscall_dispatch 

    mov rsp, rdi

    pop r15 
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    pop rsp 

    swapgs

    sysretq

; fn copy_from_user(dst, src, len)
copy_from_user:
    mov rax, rsi                
    add rax, rdx                
    mov r8, 0xFFFF800000000000  ; check if address + len encroaches kernel addr space 
    cmp rax, r8
    jae .fail_boundary

    mov rcx, rdx                ; mov len into rcx for movsb
    
    stac                        ; disable smap 
.copy_fail_point:
    rep movsb 
    clac                        ; reenable smap

    mov rax, 1                  ; success (return true)
    ret 

.fail_boundary: 
    mov rax, 0 
    ret

.recover_target: 
    clac                        ; page fault handle jumps here to recover from failure (reenable clac)
    mov rax, 0
    ret

section .extable
    align 8 
    dq .copy_fail_point         ; check fail here
    dq .recover_target          ; if fail, jump here
