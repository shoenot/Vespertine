use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::arch::x86_64::io::{inb, outb};
use crate::arch::x86_64::{IO_APIC, apic::ioapic};
use crate::kernel::acpi;
use crate::util::bitwise::{set_bit, unset_bit};

static KEYBOARD_GSI: AtomicUsize = AtomicUsize::new(1);
static EDGE: AtomicBool = AtomicBool::new(true);
static ACTIVE_HIGH: AtomicBool = AtomicBool::new(true);

const IDT_VECTOR: u8 = 33;

fn check_madt_overrides() {
    let rsdp = acpi::rsdp::Rsdp::get();
    let sdt = acpi::sdt::SDTArray::get(rsdp.get_table());
    let madt = acpi::madt::parse_madt(&sdt);
    let iso = madt.overrides;
    for entry in iso {
        if entry.source == 1 {
            KEYBOARD_GSI.store(entry.gsi as usize, Ordering::Relaxed);
            if entry.flags & 0b11 == 3 { ACTIVE_HIGH.store(false, Ordering::Relaxed); }
            if entry.flags & 0b1100 == 11 { EDGE.store(false, Ordering::Relaxed); }
        }
    }
}

pub fn init_keyboard_irq() {
    check_madt_overrides();
    IO_APIC.lock().set_entry(KEYBOARD_GSI.load(Ordering::Relaxed) as u32,
                             IDT_VECTOR,
                             0,
                             false,
                             ACTIVE_HIGH.load(Ordering::Relaxed),
                             EDGE.load(Ordering::Relaxed));
    unsafe {
        outb(0x64, 0x20);
        let mut config = inb(0x60);
        config = set_bit(config, 0);
        config = unset_bit(config, 4);
        outb(0x64, 0x60);
        outb(0x60, config);
    }
}
