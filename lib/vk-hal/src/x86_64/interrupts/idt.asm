[bits 64]
extern interrupt_dispatch

; Non error code pushing interrupts
; Dummy error code pushed onto stack
%macro ISR_NOERRCODE 1
    global isr_stub_%1 
isr_stub_%1:
    push qword 0
    push qword %1
    jmp common_interrupt_handler
%endmacro

; Error code pushing interrupts
%macro ISR_ERRCODE 1
    global isr_stub_%1
isr_stub_%1:
    push qword %1
    jmp common_interrupt_handler
%endmacro

section .text

common_interrupt_handler:
    ; check if we came from user mode by testing the rpl of the saved code selector
    test qword [rsp + 24], 3
    jz .no_swapgs_in
    swapgs
.no_swapgs_in:

    push r15
    push r14
    push r13
    push r12
    push r11
    push r10
    push r9
    push r8
    push rbp
    push rdi
    push rsi
    push rdx
    push rcx
    push rbx
    push rax

    mov rdi, rsp 
   
    cld
    call interrupt_dispatch

    pop rax
    pop rbx
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rbp
    pop r8
    pop r9
    pop r10
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15

    test qword [rsp + 24], 3
    jz .no_swapgs_out
    swapgs
.no_swapgs_out:

    add rsp, 16 ; Clean up interrupt_number and error_code
    iretq

ISR_NOERRCODE 0   ; Divide by Zero
ISR_NOERRCODE 1   ; Debug
ISR_NOERRCODE 2   ; Non-Maskable Interrupt
ISR_NOERRCODE 3   ; Breakpoint
ISR_NOERRCODE 4   ; Overflow
ISR_NOERRCODE 5   ; Bound Range Exceeded
ISR_NOERRCODE 6   ; Invalid Opcode
ISR_NOERRCODE 7   ; Device Not Available
ISR_ERRCODE   8   ; Double Fault
ISR_NOERRCODE 9   ; Coprocessor Segment Overrun (Legacy)
ISR_ERRCODE   10  ; Invalid TSS
ISR_ERRCODE   11  ; Segment Not Present
ISR_ERRCODE   12  ; Stack-Segment Fault
ISR_ERRCODE   13  ; General Protection Fault
ISR_ERRCODE   14  ; Page Fault
ISR_NOERRCODE 15  ; Reserved
ISR_NOERRCODE 16  ; x87 Floating-Point Exception
ISR_ERRCODE   17  ; Alignment Check
ISR_NOERRCODE 18  ; Machine Check
ISR_NOERRCODE 19  ; SIMD Floating-Point Exception
ISR_NOERRCODE 20  ; Virtualization Exception
ISR_ERRCODE   21  ; Control Protection Exception
ISR_NOERRCODE 22  ; Reserved
ISR_NOERRCODE 23  ; Reserved
ISR_NOERRCODE 24  ; Reserved
ISR_NOERRCODE 25  ; Reserved
ISR_NOERRCODE 26  ; Reserved
ISR_NOERRCODE 27  ; Reserved
ISR_NOERRCODE 28  ; Hypervisor Injection Exception
ISR_ERRCODE   29  ; VMM Communication Exception
ISR_ERRCODE   30  ; Security Exception
ISR_NOERRCODE 31  ; Reserved

%assign i 32
%rep 224
    ISR_NOERRCODE i 
    %assign i i+1
%endrep

section .data
    align 8
    global isr_stub_table
isr_stub_table:
    %assign i 0
    %rep 256
        dq isr_stub_%[i]
        %assign i i+1
    %endrep
