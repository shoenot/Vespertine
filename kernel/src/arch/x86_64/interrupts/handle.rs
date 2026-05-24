use core::arch::asm;
use core::sync::atomic::Ordering;

use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::arch::x86_64::cpu::core::get_core_data;
use crate::arch::x86_64::interrupts::idt::InterruptStackFrame;
use crate::arch::x86_64::interrupts::shootdown::SHOOTDOWN_INFO;
use crate::arch::x86_64::io;
use crate::core::thread::tcb::ThreadState;
use crate::drivers::keyboard;
use crate::klogln;
use crate::memory::handle_page_fault;
use crate::memory::paging::flush_tlb;

pub(in crate::arch::x86_64::interrupts) fn page_fault_handler(frame: &mut InterruptStackFrame) {
    let cr2: u64;
    unsafe {
        asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
    }

    match handle_page_fault(cr2 as usize, frame.error_code as usize) {
        Ok(_) => {},
        Err(e) => {
            if crate::arch::x86_64::interrupts::extable::fixup_exception(frame) {
                return;
            }
            klogln!("");
            klogln!("!------------- PAGE FAULT DIAGNOSTICS -------------!");
            klogln!("Faulting Address (CR2): {:#018X}", cr2);
            klogln!("Instruction Pointer (RIP): {:#018X}", frame.instruction_pointer);
            klogln!("Error Code: {:#018X}", frame.error_code);
            klogln!("Stack Frame Dump:");
            klogln!("  RAX: {:#018X} | RBX: {:#018X}", frame.rax, frame.rbx);
            klogln!("  RCX: {:#018X} | RDX: {:#018X}", frame.rcx, frame.rdx);
            klogln!("  RSI: {:#018X} | RDI: {:#018X}", frame.rsi, frame.rdi);
            klogln!("  RBP: {:#018X} | RSP: {:#018X}", frame.rbp, frame.stack_pointer);
            klogln!("  R8 : {:#018X} | R9 : {:#018X}", frame.r8, frame.r9);
            klogln!("  R10: {:#018X} | R11: {:#018X}", frame.r10, frame.r11);
            klogln!("  R12: {:#018X} | R13: {:#018X}", frame.r12, frame.r13);
            klogln!("  R14: {:#018X} | R15: {:#018X}", frame.r14, frame.r15);
            klogln!("  CS : {:#06X} | SS : {:#06X} | RFLAGS: {:#018X}", frame.code_segment, frame.stack_segment, frame.cpu_flags);
            klogln!("!--------------------------------------------------!");

            panic!("Fatal unhandled page fault: {:?}", e);
        }
    }
}


pub(in crate::arch::x86_64::interrupts) fn gpf_handler(frame: &mut InterruptStackFrame) {
    klogln!("General Protection Fault.\nError Code: {:#X}\nStack Frame:\n{:#?}", frame.error_code, frame);
    crate::hcf();
}

pub(in crate::arch::x86_64::interrupts) fn unexpected_interrupt_handler(frame: &mut InterruptStackFrame) {
    klogln!("Unexpected Interrupt.\nStack Frame:\n{:#?}", frame);
}

pub(in crate::arch::x86_64::interrupts) fn timer_interrupt_handler() {
    let core_data = get_core_data();
    core_data.apic_mode.eoi();

    if core_data.scheduler.idle_thread.is_null() {
        core_data.apic_mode.arm_oneshot(100_000);
        return;
    } 
    unsafe {
        let td_tcb_ptr = (*core_data).timer_daemon_tcb;
        if !td_tcb_ptr.is_null() {
            // In the new centralized timer model, we always wake the daemon
            // to check if it was a callout or a quantum expiry.
            if (*td_tcb_ptr).state == ThreadState::Blocked {
                (*td_tcb_ptr).state = ThreadState::Ready;
                core_data.scheduler.push(td_tcb_ptr);
            }
        }
    }

    core_data.scheduler.schedule();
}

pub(in crate::arch::x86_64::interrupts) fn ipi_handler() {
    let core_data = get_core_data();
    core_data.apic_mode.eoi();
    core_data.scheduler.schedule();
}

pub(in crate::arch::x86_64::interrupts) fn keyboard_irq_handler() {
    let core_data = get_core_data();
    core_data.apic_mode.eoi();

    // crate::drivers::serial::log_to_serial("KB INT\n");

    for _ in 0..256 {
        if unsafe { (io::inb(0x64) & 0x1) == 0 } {
            break;
        }
        keyboard::push_scancode(unsafe { io::inb(0x60) });
    }
}

pub(in crate::arch::x86_64::interrupts) fn shootdown_handler() {
    let addr = SHOOTDOWN_INFO.addr.load(Ordering::Acquire);
    flush_tlb(addr as u64);
    SHOOTDOWN_INFO.counter.fetch_sub(1, Ordering::Release);
}
