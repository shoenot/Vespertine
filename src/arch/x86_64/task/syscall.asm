extern syscall_dispatch
global _syscall_entry 
global copy_from_user
global copy_to_user

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

    o64 sysret

; fn copy_from_user(dst, src, len)
copy_from_user:
    mov rax, rsi                
    add rax, rdx                
    mov r8, 0xFFFF800000000000  ; check if address + len encroaches kernel addr space 
    cmp rax, r8
    jae .fail_boundary_from

    mov rcx, rdx                ; mov len into rcx for movsb
    
    stac                        ; disable smap 
.copy_from_fail_point:
    rep movsb 
    clac                        ; reenable smap

    mov rax, 1                  ; success (return true)
    ret 

.fail_boundary_from: 
    mov rax, 0 
    ret

.recover_target: 
    clac                        ; page fault handle jumps here to recover from failure (reenable clac)
    mov rax, 0
    ret

; fn copy_to_user(dst: *mut u8, src: *const u8, len: usize) -> bool 
copy_to_user:
    mov rax, rdi
    add rax, rdx
    mov r8, 0xFFFF800000000000
    cmp rax, r8
    jae .fail_boundary_to

    mov rcx, rdx 

    stac
.copy_to_fail_point:
    rep movsb
    clac

    mov rax, 1
    ret

.fail_boundary_to:
    mov rax, 0
    ret 

section .extable
    align 8 
    ; entry 1: copy_from_user
    dq copy_from_user.copy_from_fail_point         ; check fail here
    dq copy_from_user.recover_target          ; if fail, jump here
    ; entry 2: copy_from_user
    dq copy_to_user.copy_to_fail_point         ; check fail here
    dq copy_from_user.recover_target          ; if fail, jump here
