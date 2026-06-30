pub mod alloc;
pub mod trap;
pub mod extable;

use crate::interrupts::alloc::{
    ArchMsiFns,
    init_arch,
};
use crate::klogln;

fn arch_program_msi(vector: u8, _target_apic_id: u32, _data: u32) {
    klogln!("[WARN] arch_program_msi() not implemented - vector {}", vector);
}

fn arch_free_vector(vector: u8) {
    klogln!("[WARN] arch_free_vector() not implemented - vector {}", vector);
}

pub fn init() {
    hal::interrupts::init(trap::dispatch);
    klogln!("[SUCCESS] IDT Loaded.");

    init_arch(ArchMsiFns {
        program_msi: arch_program_msi,
        free_vector: arch_free_vector,
        register_irq_entry: trap::register_irq_entry,
    });
}
