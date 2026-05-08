use core::arch::asm;
use lazy_static::lazy_static;
use crate::drivers::serial::{log_to_serial, log_u64_to_serial};
use crate::hcf;
use crate::GLOBAL_VMM;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct InterruptDescriptor {
    address_low: u16,
    selector: u16,
    ist: u8,
    flags: u8,
    address_mid: u16,
    address_high: u32,
    reserved: u32,
}

impl InterruptDescriptor {
    const FLAGS_INTERRUPT_GATE: u8 = 0x8E;
    const KERNEL_CODE_SEGMENT: u16 = 0x08;
    
    pub fn new(handler_address: u64) -> Self {
        InterruptDescriptor { 
            address_low: handler_address as u16, 
            selector: Self::KERNEL_CODE_SEGMENT, 
            ist: 0, 
            flags: Self::FLAGS_INTERRUPT_GATE, 
            address_mid: (handler_address >> 16) as u16, 
            address_high: (handler_address >> 32) as u32, 
            reserved: 0, 
        } 
    }
}

#[repr(C, packed)]
struct IDTDescriptor {
    size: u16,
    address: u64
}

unsafe extern "C" {
    static isr_stub_table: [u64; 256];
}

lazy_static! {
    static ref IDT: [InterruptDescriptor; 256] = {
        let mut idt = [InterruptDescriptor::new(0); 256];

        for i in 0..256 {
            unsafe {
                let handler_addr = isr_stub_table[i];
                idt[i] = InterruptDescriptor::new(handler_addr);
            }
        }
        idt 
    };
}

pub fn init_idt() {
    let idt_address = &*IDT as *const [InterruptDescriptor; 256] as u64;
    
    let idt_ptr = IDTDescriptor {
        size: (core::mem::size_of::<[InterruptDescriptor; 256]>() - 1) as u16,
        address: idt_address,
    };

    unsafe {
        asm!(
            "lidt ({ptr})",
            ptr = in(reg) &idt_ptr,
            options(att_syntax, nostack, preserves_flags)
        );

        asm!(
            "sti",
            options(att_syntax, nostack, preserves_flags)
        )
    }

    log_to_serial("IDT INIT OK\n");
}

#[repr(C)]
#[derive(Debug)]
pub struct InterruptStackFrame {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,

    pub interrupt_number: u64,
    pub error_code: u64,

    pub instruction_pointer: u64,
    pub code_segment: u64,
    pub cpu_flags: u64,
    pub stack_pointer: u64,
    pub stack_segment: u64,
}

#[unsafe(no_mangle)]
pub extern "C" fn interrupt_dispatch(frame: &mut InterruptStackFrame) {
    match frame.interrupt_number {
        13 => {
            log_to_serial("general protection fault.\n");
            gpf_handler(&frame);
        },
        14 => {
            log_to_serial("\npage fault.\n");
            page_fault_handler(&frame);
        },
        15 => log_to_serial("unexpected interrupt.\n"),
        _ => {},
    }
}

fn gpf_handler(frame: &InterruptStackFrame) {
    log_to_serial("Error Code: ");
    log_u64_to_serial(frame.error_code);
    log_to_serial(" Instruction Pointer: ");
    log_u64_to_serial(frame.instruction_pointer);
    hcf();
}

fn read_cr2() -> u64 {
    let cr2: u64;
    unsafe {
        asm!("movq %cr2, {0}", out(reg) cr2, options(att_syntax, nostack, preserves_flags));
    };
    cr2
}

fn page_fault_handler(frame: &InterruptStackFrame) {
    let addr = read_cr2() as usize;
    let error_code = frame.error_code as usize;
    let mut vmm = GLOBAL_VMM.lock();

    let fixed = vmm.handle_page_fault(addr, error_code);

    if !fixed {
        panic!(
            "PAGE FAULT EXCEPTION\nAT ADDRESS: {:#X}\nError Code: {:#b}\n{:#?}",
            addr, error_code, frame
        )
    }
}
