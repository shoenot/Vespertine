section .data 
    obj_name db "Objects"
    obj_name_len equ $ - obj_name

    clock_name db "Clock"
    clock_name_len equ $ - clock_name

    align 8
    inv_lookup_obj:
        dd 3            ; Invocation::Directory
        dd 0            ; 4 bytes padding
        dd 2            ; DirectoryOp::Lookup
        dd 0            
        dq obj_name
        dq obj_name_len

    align 8
    inv_lookup_clock:
        dd 3
        dd 0 
        dd 2
        dd 0 
        dq clock_name
        dq clock_name_len

    align 8
    inv_timestamp:
        dd 10           ; Invocation::Clock
        dd 0            
        dd 0            ; ClockOp::GetTimestamp
        dd 0 

section .text 
global _start 

_start:
    mov rax, 0
    mov rdi, 0
    mov rsi, inv_lookup_obj
    syscall

    cmp rax, 0
    jne exit_prog

    mov rdi, rdx
    mov rax, 0
    mov rsi, inv_lookup_clock
    syscall

    cmp rax, 0
    jne exit_prog

    mov rdi, rdx
    mov rax, 0
    mov rsi, inv_timestamp
    syscall

exit_prog: 
    mov rax, 2
    syscall

    hlt
