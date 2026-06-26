use alloc::vec::Vec;
use vespertine_abi::{PROC_FAULT_GENERAL_PROTECTION, PROC_FAULT_INVALID_OPCODE, PROC_FAULT_PAGE};
use core::arch::asm;
use core::sync::atomic::Ordering;

use crate::arch::{disable_interrupts, hcf};
use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::arch::x86_64::cpu::core::get_core_data;
use crate::arch::x86_64::interrupts::extable::fixup_exception;
use crate::arch::x86_64::interrupts::idt::InterruptStackFrame;
use crate::arch::x86_64::interrupts::shootdown::SHOOTDOWN_INFO;
use crate::arch::x86_64::io;
use crate::core::object::models::process::ProcTermination;
use crate::core::sync::TicketLock;
use crate::core::thread::dispatch::wake_thread;
use crate::core::thread::get_current_process;
use crate::core::thread::schedule::ScheduleReason;
use crate::core::time::get_time;
use crate::drivers::keyboard;
use crate::klogln;
use crate::memory::handle_page_fault;
use crate::memory::paging::flush_tlb;

pub(in crate::arch::x86_64) static IRQ_HANDLERS: TicketLock<Vec<Option<(extern "C" fn(arg: usize), usize)>>> = TicketLock::new(Vec::new());

fn frame_from_user(frame: &InterruptStackFrame) -> bool {
    frame.code_segment & 0x3 == 0x3
}

fn terminate_user_fault(frame: &InterruptStackFrame, code: u32, detail: usize) -> ! {
    if let Some(proc) = get_current_process() {
        proc.request_terminate(ProcTermination::faulted(code, detail));
    } else {
        panic!("user fault without current process: {:#?}", frame);
    }
    get_core_data().scheduler.terminate_current_thread(code)
}

pub(in crate::arch::x86_64::interrupts) fn page_fault_handler(frame: &mut InterruptStackFrame) {
    let cr2: u64;
    unsafe {
        asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
    }

    let int_state = (frame.cpu_flags & (1 << 9)) != 0;
    if int_state {
        crate::arch::enable_interrupts();
    }

    match handle_page_fault(cr2 as usize, frame.error_code as usize) {
        Ok(_) => {
            if int_state {
                disable_interrupts();
            }
        }
        Err(e) => {
            if fixup_exception(frame) {
                if int_state {
                    disable_interrupts();
                }
                return;
            }

            if frame_from_user(frame) {
                klogln!(
                    "terminating process after user page fault: 
                    rip: {:#018X}, addr: {:#018X}, error: {:#018X}, fault: {:?}",
                    frame.instruction_pointer,
                    cr2,
                    frame.error_code,
                    e
                );
                terminate_user_fault(frame, PROC_FAULT_PAGE, cr2 as usize);
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
      if frame_from_user(frame) {
          klogln!(
              "terminating process after user general protection fault: rip: {:#018X} error: {:#018X}",
              frame.instruction_pointer,
              frame.error_code
          );

          terminate_user_fault(frame, PROC_FAULT_GENERAL_PROTECTION, frame.instruction_pointer as usize);
      }
    klogln!("General Protection Fault.\nError Code: {:#X}\nStack Frame:\n{:#?}", frame.error_code, frame);
    hcf();
}

pub(in crate::arch::x86_64::interrupts) fn invalid_opcode_handler(frame: &mut InterruptStackFrame) {
    if frame_from_user(frame) {
        klogln!(
            "terminating process after user invalid opcode: rip: {:#018X}",
            frame.instruction_pointer
        );

        terminate_user_fault(frame, PROC_FAULT_INVALID_OPCODE, frame.instruction_pointer as usize);
    }

    panic!("INVALID OPCODE (#UD): {:#?}", frame);
}

pub(in crate::arch::x86_64::interrupts) fn unexpected_interrupt_handler(frame: &mut InterruptStackFrame) {
    klogln!("Unexpected Interrupt.\nStack Frame:\n{:#?}", frame);
}

pub(in crate::arch::x86_64::interrupts) fn timer_interrupt_handler() {
    let core_data = get_core_data();

    if core_data.scheduler.idle_thread.is_null() {
        core_data.apic_mode.arm_oneshot(100_000);
        return;
    }
    let td_tcb_ptr = core_data.timer_daemon_tcb;
    if !td_tcb_ptr.is_null() {
        core_data.timer_daemon_awoken.store(true, Ordering::Release);
        wake_thread(td_tcb_ptr);
    }

    let now = get_time();
    let current = core_data.scheduler.current_thread;

    let reason = if !current.is_null() && unsafe { (*current).quantum_expiry <= now } {
        ScheduleReason::QuantumExpired
    } else {
        ScheduleReason::TimerEvent
    };

    core_data.scheduler.schedule(reason);
}

pub(in crate::arch::x86_64::interrupts) fn ipi_handler() { get_core_data().scheduler.schedule(ScheduleReason::RescheduleIpi); }

pub(in crate::arch::x86_64::interrupts) fn keyboard_irq_handler() {
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
