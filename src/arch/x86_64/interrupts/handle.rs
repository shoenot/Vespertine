use core::arch::asm;

use crate::arch::x86_64::apic::lapic::{
    ApicDriver,
    ApicMode,
};
use crate::arch::x86_64::cpu::core::get_core_data;
use crate::arch::x86_64::interrupts::idt::InterruptStackFrame;
use crate::kernel::thread::schedule::DEFAULT_QUANTUM;
use crate::kernel::thread::tcb::ThreadState;
use crate::kernel::thread::workqueue::{
    WorkItem,
    WorkQueue,
    worker_thread,
};
use crate::kernel::time::{
    arm_sleep_ns,
    arm_sleep_ticks,
    get_time,
};
use crate::klogln;
use crate::memory::GLOBAL_VMM;

pub(in crate::arch::x86_64::interrupts) fn page_fault_handler(frame: &mut InterruptStackFrame) {
    let cr2: u64;
    unsafe {
        asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
    }

    let mut vmm = GLOBAL_VMM.lock();
    if !vmm.handle_page_fault(cr2 as usize, frame.error_code as usize) {
        panic!("FATAL: Unhandled Page Fault!");
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
    let current_time = get_time();

    match &core_data.apic_mode {
        ApicMode::XApic(apic) => apic.eoi(),
        ApicMode::X2Apic(apic) => apic.eoi(),
    }

    let sched = &mut core_data.scheduler;

    unsafe {
        while !sched.sleep_queue_head.is_null() {
            let sleeping_thread = sched.sleep_queue_head;

            if (*sleeping_thread).wake_time > current_time {
                break;
            }

            sched.sleep_queue_head = (*sleeping_thread).next;
            (*sleeping_thread).next = core::ptr::null_mut();

            (*sleeping_thread).state = ThreadState::Ready;
            sched.push(sleeping_thread);
        }
    }

    if !sched.sleep_queue_head.is_null() {
        let next_wake = unsafe { (*sched.sleep_queue_head).wake_time };

        let delta_ticks = next_wake.saturating_sub(current_time);

        arm_sleep_ticks(delta_ticks);
    } else {
        arm_sleep_ns(DEFAULT_QUANTUM);
    }

    sched.schedule();
}

pub(in crate::arch::x86_64::interrupts) fn ipi_handler() {
    let core_data = get_core_data();
    core_data.apic_mode.eoi();
    klogln!(">>> Core {} forcefully woken up by an IPI <<<", core_data.lapic_id);
    core_data.scheduler.schedule();
}
