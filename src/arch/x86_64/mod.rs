pub mod apic;
pub mod cpuid;
pub mod interrupts;
pub mod io;
pub mod timer;
pub mod task;
pub mod cpu;

use apic::{
    ioapic::*,
    lapic::*,
};

use crate::{
    kernel::{
        memory::PAGER,
        sync::TicketLock,
    },
    klog,
    klogln,
};

pub static LOCAL_APIC: TicketLock<LocalAPIC> = TicketLock::new(LocalAPIC { base_addr: 0 });
pub static IO_APIC: TicketLock<IOApic> = TicketLock::new(IOApic { base_addr: 0, gsi_base: 0 });

pub fn init_interrupts() {
    klog!("INITIATING GDT...");
    interrupts::gdt::init_gdt();
    klogln!("GDT INIT OK.");
    klog!("INITIATING IDT...");
    interrupts::idt::init_idt();
    klogln!("IDT INIT OK.");
}

pub fn init_apic() {
    let mut lapic = LOCAL_APIC.lock();
    let mut ioapic = IO_APIC.lock();

    map_lapic_memory();
    lapic.init();

    let (ioapic_base, ioapic_gsi_base) = get_ioapic_addrs();
    map_ioapic_memory(ioapic_base as u64);
    ioapic.init(ioapic_base, ioapic_gsi_base);
    ioapic.mask_all();
}

fn map_lapic_memory() {
    let apic_phys = get_apic_base() as u64;
    let mut pager = PAGER.lock();
    pager.map_mmio_addr(apic_phys).unwrap();
    drop(pager);
}

fn map_ioapic_memory(base_addr: u64) {
    let ioapic_phys = base_addr as u64;
    let mut pager = PAGER.lock();
    pager.map_mmio_addr(ioapic_phys).unwrap();
    drop(pager);
}

