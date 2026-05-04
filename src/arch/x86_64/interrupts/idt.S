.altmacro

.macro ISR_NOERRCODE num
    .global isr_stub_\num 
isr_stub_\num:
    pushq $0
    pushq $\num
    jmp common_interrupt_handler
.endm
.macro ISR_ERRCODE num
    .global isr_stub_\num 
isr_stub_\num:
    pushq $\num
    jmp common_interrupt_handler
.endm

.macro PTR_ENTRY num
    .quad isr_stub_\num 
.endm 

.text
common_interrupt_handler:
    pushq %r15
    pushq %r14
    pushq %r13
    pushq %r12
    pushq %r11
    pushq %r10
    pushq %r9
    pushq %r8
    pushq %rbp
    pushq %rdi
    pushq %rsi
    pushq %rdx
    pushq %rcx
    pushq %rbx
    pushq %rax

    movq %rsp, %rdi 
   
    cld
    call interrupt_dispatch

    popq %rax
    popq %rbx
    popq %rcx
    popq %rdx
    popq %rsi
    popq %rdi
    popq %rbp
    popq %r8
    popq %r9
    popq %r10
    popq %r11
    popq %r12
    popq %r13
    popq %r14
    popq %r15

    addq $16, %rsp

    iretq

.extern interrupt_dispatch 

ISR_NOERRCODE 0   // Divide by Zero
ISR_NOERRCODE 1   // Debug
ISR_NOERRCODE 2   // Non-Maskable Interrupt
ISR_NOERRCODE 3   // Breakpoint
ISR_NOERRCODE 4   // Overflow
ISR_NOERRCODE 5   // Bound Range Exceeded
ISR_NOERRCODE 6   // Invalid Opcode
ISR_NOERRCODE 7   // Device Not Available
ISR_ERRCODE   8   // Double Fault
ISR_NOERRCODE 9   // Coprocessor Segment Overrun (Legacy)
ISR_ERRCODE   10  // Invalid TSS
ISR_ERRCODE   11  // Segment Not Present
ISR_ERRCODE   12  // Stack-Segment Fault
ISR_ERRCODE   13  // General Protection Fault
ISR_ERRCODE   14  // Page Fault
ISR_NOERRCODE 15  // Reserved
ISR_NOERRCODE 16  // x87 Floating-Point Exception
ISR_ERRCODE   17  // Alignment Check
ISR_NOERRCODE 18  // Machine Check
ISR_NOERRCODE 19  // SIMD Floating-Point Exception
ISR_NOERRCODE 20  // Virtualization Exception
ISR_ERRCODE   21  // Control Protection Exception
ISR_NOERRCODE 22  // Reserved
ISR_NOERRCODE 23  // Reserved
ISR_NOERRCODE 24  // Reserved
ISR_NOERRCODE 25  // Reserved
ISR_NOERRCODE 26  // Reserved
ISR_NOERRCODE 27  // Reserved
ISR_NOERRCODE 28  // Hypervisor Injection Exception
ISR_ERRCODE   29  // VMM Communication Exception
ISR_ERRCODE   30  // Security Exception
ISR_NOERRCODE 31  // Reserved

.set i, 32
.rept 224
    ISR_NOERRCODE %i 
    .set i, i + 1
.endr

.data
.global isr_stub_table
.align 8
isr_stub_table:
.set i, 0
.rept 256
    PTR_ENTRY %i
    .set i, i + 1
.endr 
