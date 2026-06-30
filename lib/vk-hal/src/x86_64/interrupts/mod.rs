use core::arch::asm;

use common::once::KernelOnceCell as OnceCell;

use crate::x86_64::apic::lapic::send_eoi;

pub const TIMER_VECTOR: u8 = 35;
pub const RESCHEDULE_IPI_VECTOR: u8 = 40;
pub const TLB_SHOOTDOWN_IPI_VECTOR: u8 = 41;

#[repr(C)]
#[derive(Debug)]
pub struct TrapFrame {
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

impl TrapFrame {
    pub fn vector(&self) -> u8 {
        self.interrupt_number as u8
    }

    pub fn error_code(&self) -> usize {
        self.error_code as usize
    }

    pub fn user_mode(&self) -> bool {
        self.code_segment & 0x3 == 0x3
    }

    pub fn interrupts_enabled(&self) -> bool {
        self.cpu_flags & (1 << 9) != 0
    }

    pub fn is_irq(&self) -> bool {
        self.interrupt_number >= 32
    }
}

pub type TrapHandler = extern "C" fn(&mut TrapFrame);

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

    fn new(handler_address: u64) -> Self {
        Self {
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
struct IdtDescriptor {
    size: u16,
    address: u64,
}

unsafe extern "C" {
    static isr_stub_table: [u64; 256];
}

static IDT: OnceCell<[InterruptDescriptor; 256]> = OnceCell::new();
static TRAP_HANDLER: OnceCell<TrapHandler> = OnceCell::new();

pub fn init(handler: TrapHandler) {
    TRAP_HANDLER.get_or_init(|| handler);

    IDT.get_or_init(|| {
        let mut idt = [InterruptDescriptor::new(0); 256];
        for i in 0..256 {
            unsafe {
                idt[i] = InterruptDescriptor::new(isr_stub_table[i]);
            }
        }
        idt
    });
    load_local();
}

pub fn load_local() {
    let idt_ptr = IdtDescriptor {
        size: (core::mem::size_of::<[InterruptDescriptor; 256]>() - 1) as u16,
        address: &*IDT as *const [InterruptDescriptor; 256] as u64,
    };
    unsafe {
        asm!(
            "lidt [{ptr}]",
            ptr = in(reg) &idt_ptr,
            options(nostack, preserves_flags),
        );
    }
}

#[unsafe(no_mangle)]
extern "C" fn interrupt_dispatch(frame: &mut TrapFrame) {
    if frame.is_irq() {
        send_eoi();
    }
    let handler = *TRAP_HANDLER;
    handler(frame);
}

#[inline]
pub fn disable_interrupts() {
    unsafe {
        asm!("cli", options(nostack));
    }
}

#[inline]
pub fn enable_interrupts() {
    unsafe {
        asm!("sti", options(nostack));
    }
}

#[inline]
pub fn interrupts_enabled() -> bool {
    let rflags: usize;
    unsafe {
        asm!(
            "pushf",
            "pop {}",
            out(reg) rflags,
            options(nomem, preserves_flags),
        );
    }
    (rflags & (1 << 9)) != 0
}

#[inline]
pub fn page_fault_address() -> usize {
    let cr2: usize;
    unsafe {
        asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
    }
    cr2
}
