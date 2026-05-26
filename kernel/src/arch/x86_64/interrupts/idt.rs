use core::arch::asm;

use crate::arch::hcf;
use crate::arch::x86_64::apic::lapic::send_apic_eoi;
use crate::arch::x86_64::interrupts::handle;
use crate::core::sync::KernelOnceCell;
use crate::klogln;

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

    pub(super) fn new(handler_address: u64) -> Self {
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
    address: u64,
}

unsafe extern "C" {
    static isr_stub_table: [u64; 256];
}

static IDT: KernelOnceCell<[InterruptDescriptor; 256]> = KernelOnceCell::new();

pub(in crate::arch::x86_64) fn init_idt() {
    IDT.get_or_init(|| {
        let mut idt = [InterruptDescriptor::new(0); 256];

        for i in 0..256 {
            unsafe {
                let handler_addr = isr_stub_table[i];
                idt[i] = InterruptDescriptor::new(handler_addr);
            }
        }
        idt
    });

    let idt_address = &*IDT as *const [InterruptDescriptor; 256] as u64;

    let idt_ptr = IDTDescriptor { size: (core::mem::size_of::<[InterruptDescriptor; 256]>() - 1) as u16, address: idt_address };

    unsafe {
        asm!(
            "lidt [{ptr}]",
            ptr = in(reg) &idt_ptr,
            options(nostack, preserves_flags)
        );
    }
}

pub fn load_idt() {
    let idt_ptr =
        IDTDescriptor { size: (core::mem::size_of::<[InterruptDescriptor; 256]>() - 1) as u16, address: &*IDT as *const _ as u64 };
    unsafe {
        asm!(
            "lidt [{ptr}]",
            ptr = in(reg) &idt_ptr,
            options(nostack, preserves_flags)
        );
    }
}

#[repr(C)]
#[derive(Debug)]
pub(crate) struct InterruptStackFrame {
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
extern "C" fn interrupt_dispatch(frame: &mut InterruptStackFrame) {
    match frame.interrupt_number {
        6 => panic!("INVALID OPCODE (#UD): {:#?}", frame),
        8 => panic!("DOUBLE FAULT: {:#?}", frame),
        13 => handle::gpf_handler(frame),
        14 => handle::page_fault_handler(frame),
        15 => handle::unexpected_interrupt_handler(frame),
        33 => handle::keyboard_irq_handler(),
        35 => handle::timer_interrupt_handler(),
        64 => handle::ipi_handler(),
        65 => handle::shootdown_handler(),
        _ => {
            if frame.interrupt_number >= 32 {
                klogln!("unshandled exception: {}", frame.interrupt_number);
                hcf()
            }
        }
    }
}
