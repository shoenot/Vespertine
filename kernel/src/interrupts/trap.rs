use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use hal::io;
use crate::memory::shootdown::service_pending_shootdown;
use crate::interrupts::extable::fixup_exception;
use hal::cpu::halt_loop;
use hal::interrupts::{
    TrapFrame,
    disable_interrupts,
    enable_interrupts,
    page_fault_address,
};
use vespertine_abi::{
    PROC_FAULT_GENERAL_PROTECTION,
    PROC_FAULT_INVALID_OPCODE,
    PROC_FAULT_PAGE,
};

use crate::core::cpu::current_core_mut;
use crate::core::object::models::process::ProcTermination;
use crate::core::sync::TicketLock;
use crate::core::thread::dispatch::wake_thread;
use crate::core::thread::get_current_process;
use crate::core::thread::schedule::ScheduleReason;
use crate::core::time::get_time;
use crate::drivers::keyboard;
use crate::klogln;
use crate::memory::handle_page_fault;

pub static IRQ_HANDLERS: TicketLock<Vec<Option<(extern "C" fn(arg: usize), usize)>>> = TicketLock::new(Vec::new());

pub extern "C" fn register_irq_entry(vector: u8, handler: extern "C" fn(arg: usize), arg: usize) {
    let mut table = IRQ_HANDLERS.lock();

    if vector as usize >= table.len() {
        table.resize(vector as usize + 1, None);
    }

    table[vector as usize] = Some((handler, arg));
}

pub extern "C" fn dispatch(frame: &mut TrapFrame) {
    match frame.vector() {
        6 => invalid_opcode_handler(frame),
        8 => panic!("DOUBLE FAULT: {:#?}", frame),
        13 => gpf_handler(frame),
        14 => page_fault_handler(frame),
        15 => unexpected_interrupt_handler(frame),
        33 => keyboard_irq_handler(),
        hal::interrupts::TIMER_VECTOR => timer_interrupt_handler(),
        hal::interrupts::RESCHEDULE_IPI_VECTOR => ipi_handler(),
        hal::interrupts::TLB_SHOOTDOWN_IPI_VECTOR => shootdown_handler(),
        _ => dispatch_dynamic_irq(frame),
    }
}

fn dispatch_dynamic_irq(frame: &mut TrapFrame) {
    if !frame.is_irq() {
        klogln!("UNHANDLED EXCEPTION: {}", frame.vector());
        return;
    }

    let handlers = IRQ_HANDLERS.lock();
    let idx = frame.vector() as usize;

    if idx < handlers.len() {
        if let Some((handler, arg)) = handlers[idx] {
            handler(arg);
        } else {
            klogln!("UNHANDLED INTERRUPT: {}", frame.vector());
        }
    } else {
        klogln!("UNHANDLED INTERRUPT: {}", frame.vector());
    }
}

fn terminate_user_fault(frame: &TrapFrame, code: u32, detail: usize) -> ! {
    if let Some(proc) = get_current_process() {
        proc.request_terminate(ProcTermination::faulted(code, detail));
    } else {
        panic!("user fault without current process: {:#?}", frame);
    }
    current_core_mut().scheduler.terminate_current_thread(code)
}

pub fn page_fault_handler(frame: &mut TrapFrame) {
    let cr2 = page_fault_address();

    let int_state = (frame.cpu_flags & (1 << 9)) != 0;
    if int_state {
        enable_interrupts();
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

            if frame.user_mode() {
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

pub fn gpf_handler(frame: &mut TrapFrame) {
    if frame.user_mode() {
        klogln!(
            "terminating process after user general protection fault: rip: {:#018X} error: {:#018X}",
            frame.instruction_pointer,
            frame.error_code
        );

        terminate_user_fault(frame, PROC_FAULT_GENERAL_PROTECTION, frame.instruction_pointer as usize);
    }
    klogln!("General Protection Fault.\nError Code: {:#X}\nStack Frame:\n{:#?}", frame.error_code, frame);
    halt_loop();
}

pub fn invalid_opcode_handler(frame: &mut TrapFrame) {
    if frame.user_mode() {
        klogln!("terminating process after user invalid opcode: rip: {:#018X}", frame.instruction_pointer);

        terminate_user_fault(frame, PROC_FAULT_INVALID_OPCODE, frame.instruction_pointer as usize);
    }

    panic!("INVALID OPCODE (#UD): {:#?}", frame);
}

pub fn unexpected_interrupt_handler(frame: &mut TrapFrame) {
    klogln!("Unexpected Interrupt.\nStack Frame:\n{:#?}", frame);
}

pub fn timer_interrupt_handler() {
    let core_data = current_core_mut();

    if core_data.scheduler.idle_thread.is_null() {
        hal::timer::arm_relative_ticks(100_000);
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

pub fn ipi_handler() { current_core_mut().scheduler.schedule(ScheduleReason::RescheduleIpi); }

pub fn keyboard_irq_handler() {
    for _ in 0..256 {
        if unsafe { (io::inb(0x64) & 0x1) == 0 } {
            break;
        }
        keyboard::push_scancode(unsafe { io::inb(0x60) });
    }
}

pub fn shootdown_handler() { service_pending_shootdown(); }
