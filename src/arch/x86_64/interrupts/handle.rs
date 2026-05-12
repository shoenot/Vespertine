use core::{arch::asm, ptr::null_mut};

use crate::{
    memory::GLOBAL_VMM,
    arch::x86_64::{
        apic::lapic::send_apic_eoi,
        interrupts::idt::InterruptStackFrame,
    },
    kernel::{
        SCHEDULER,
        thread::tcb::ThreadState, 
        time::{
            arm_sleep_ns, 
            get_time
        }
    },
    klogln,
};

// HELPERS

fn read_cr2() -> u64 {
    let cr2: u64;
    unsafe {
        asm!("mov {0}, cr2", 
            out(reg) cr2, 
            options(nostack, preserves_flags));
    };
    cr2
}

// HANDLERS

// Interrupt 13
pub(in crate::arch::x86_64::interrupts) fn gpf_handler(frame: &InterruptStackFrame) {
    panic!("General Protection Fault.\nError Code: {}\nInstruction Pointer: {:#X}\n", frame.error_code, frame.instruction_pointer);
}

// Interrupt 14
pub(in crate::arch::x86_64::interrupts) fn page_fault_handler(frame: &InterruptStackFrame) {
    let addr = read_cr2() as usize;
    let error_code = frame.error_code as usize;
    let mut vmm = GLOBAL_VMM.lock();

    let fixed = vmm.handle_page_fault(addr, error_code);

    if !fixed {
        panic!("Page Fault Exception.\nAt address: {:#X}\nError Code: {:#b}\nStack Frame:\n{:#?}", addr, error_code, frame)
    }
}

pub(in crate::arch::x86_64::interrupts) fn unexpected_interrupt_handler(frame: &InterruptStackFrame) {
    klogln!("Unexpected Interrupt.\nStack Frame:\n{:#?}", frame);
}

pub(in crate::arch::x86_64::interrupts) fn lapic_interrupt_handler() {
    let current_time = get_time();
    send_apic_eoi();

    let mut sched = SCHEDULER.lock();
    unsafe {
        while !sched.sleep_queue_head.is_null() {
            let sleeping_thread = sched.sleep_queue_head;

            if (*sleeping_thread).wake_time > current_time {
                break;
            }

            sched.sleep_queue_head = (*sleeping_thread).next;
            (*sleeping_thread).next = null_mut();

            (*sleeping_thread).state = ThreadState::Ready;
            sched.push(sleeping_thread);
        }
    }
    if !sched.sleep_queue_head.is_null() {
        let next_wake = unsafe {
            (*sched.sleep_queue_head).wake_time 
        };
        let delta_ns = next_wake.saturating_sub(get_time());
        arm_sleep_ns(delta_ns);
    } else {
        arm_sleep_ns(1_000_000_000);
    }
    sched.schedule();
    drop(sched);
}
