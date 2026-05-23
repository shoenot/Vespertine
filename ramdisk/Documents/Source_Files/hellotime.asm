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

    align 8
    inv_console_log:
        dd 4            ; Invocation::File
        dd 0            ; 4 bytes padding
        dd 1            ; FileOp::Write
        dd 0            ; 4 bytes padding
        dq 0            ; Offset
        dq 0
        dq 0

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

    mov rax, rdx 
    lea rdi, [time_buf + 24]
    xor r8, r8
    mov rcx, 10

.itoa_loop:
    xor rdx, rdx 
    div rcx 
    add rdx, 0x30
    
    dec rdi
    mov [rdi], dl

    inc r8

    test rax, rax 
    jnz .itoa_loop

    mov [inv_console_log + 24], rdi
    mov [inv_console_log + 32], r8

    mov rax, 0 
    mov rdi, 2 
    lea rsi, [inv_console_log]
    syscall

exit_prog: 
    mov rax, 2
    syscall

    hlt

section .bss
    time_buf: resb 24
