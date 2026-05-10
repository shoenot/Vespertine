use core::arch::asm;
use super::idt::InterruptStackFrame;
use core::sync::atomic::Ordering;
use super::super::apic::lapic::send_apic_eoi;
use crate::{
    GLOBAL_VMM, USE_TSC_DEADLINE, kernel::time::{self, arm_sleep_ns}, klogln
};

// HELPERS

pub fn read_cr2() -> u64 {
    let cr2: u64;
    unsafe {
        asm!("movq %cr2, {0}", out(reg) cr2, options(att_syntax, nostack, preserves_flags));
    };
    cr2
}

// HANDLERS

// Interrupt 13
pub fn gpf_handler(frame: &InterruptStackFrame) {
    panic!(
        "General Protection Fault.\nError Code: {}\nInstruction Pointer: {:#X}\n", 
        frame.error_code, 
        frame.instruction_pointer
    );
}

// Interrupt 14
pub fn page_fault_handler(frame: &InterruptStackFrame) {
    let addr = read_cr2() as usize;
    let error_code = frame.error_code as usize;
    let mut vmm = GLOBAL_VMM.lock();

    let fixed = vmm.handle_page_fault(addr, error_code);

    if !fixed {
        panic!(
            "Page Fault Exception.\nAt address: {:#X}\nError Code: {:#b}\nStack Frame:\n{:#?}",
            addr, error_code, frame
        )
    }
}

pub fn unexpected_interrupt_handler(frame: &InterruptStackFrame) {
    klogln!("Unexpected Interrupt.\nStack Frame:\n{:#?}", frame);
}

pub fn lapic_interrupt_handler() {
    send_apic_eoi();
    klogln!("poggers");
    arm_sleep_ns(1_000_000_000);
}
