pub(crate) mod extable;
pub(crate) mod handle;
pub(crate) mod idt;
pub(crate) mod shootdown;

use crate::klogln;

/// program msi/msi-x for vector
pub fn arch_program_msi(vector: u8, _target_apic_id: u32, _data: u32) {
    klogln!("[WARN] arch_program_msi() not implemented - vector {}", vector);
}

pub fn arch_free_vector(vector: u8) {
    klogln!("[WARN] arch_free_vector() not implemented - vector {}", vector);
}

pub extern "C" fn arch_register_irq_entry(vector: u8, handler: extern "C" fn(arg: usize), arg: usize) {
    let mut table = handle::IRQ_HANDLERS.lock();
    if (vector as usize) >= table.len() {
        table.resize(vector as usize + 1, None);
    }
    table[vector as usize] = Some((handler, arg));
}
