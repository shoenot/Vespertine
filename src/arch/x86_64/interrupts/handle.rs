use core::arch::asm;

use crate::arch::x86_64::io::inb;
use crate::arch::x86_64::{IO_APIC, io};
use crate::arch::x86_64::apic::lapic::{
    ApicDriver,
};
use crate::arch::x86_64::cpu::core::get_core_data;
use crate::arch::x86_64::interrupts::idt::InterruptStackFrame;
use crate::kernel::thread::tcb::ThreadState;
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
    core_data.apic_mode.eoi(); 

    if core_data.scheduler.idle_thread.is_null() {
        return;
    }

    unsafe {
        let td_tcb_ptr = (*core_data).timer_daemon_tcb;
        if !td_tcb_ptr.is_null() {
            // In the new centralized timer model, we always wake the daemon
            // to check if it was a callout or a quantum expiry.
            if (*td_tcb_ptr).state != ThreadState::Running {
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
    
    crate::drivers::serial::log_to_serial("KB INT\n");

    unsafe {
        for _ in 0..10 {
            if (io::inb(0x64) & 0x1) == 0 {
                break 
            }
            io::inb(0x60);
        }
    }
}
