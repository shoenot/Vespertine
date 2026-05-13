pub mod apic;
pub mod cpu;
pub mod cpuid;
pub mod interrupts;
pub mod io;
pub mod task;
pub mod timer;

use core::arch::asm;

use apic::ioapic::*;
use apic::lapic::get_apic_base;

use crate::kernel::sync::TicketLock;
use crate::memory::PAGER;
use crate::{
    klog,
    klogln,
};

pub static IO_APIC: TicketLock<IOApic> = TicketLock::new(IOApic { base_addr: 0, gsi_base: 0 });

pub fn init_interrupts() {
    klog!("INITIATING IDT...");
    interrupts::idt::init_idt();
    klogln!("IDT INIT OK.");
}

pub fn init_global_apics() {
    let mut ioapic = IO_APIC.lock();

    map_lapic_memory();

    let (ioapic_base, ioapic_gsi_base) = get_ioapic_addrs();
    map_ioapic_memory(ioapic_base as u64);
    ioapic.init(ioapic_base, ioapic_gsi_base);
    ioapic.mask_all();
}

pub fn map_lapic_memory() {
    let apic_phys = get_apic_base() as u64;
    let mut pager = PAGER.lock();
    pager.map_mmio_addr(apic_phys).expect("Failed to map LAPIC MMIO");
    drop(pager);
}

fn map_ioapic_memory(base_addr: u64) {
    let ioapic_phys = base_addr as u64;
    let mut pager = PAGER.lock();
    pager.map_mmio_addr(ioapic_phys).expect("Failed to map IOAPIC MMIO");
    drop(pager);
}

pub fn hcf() -> ! {
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}
