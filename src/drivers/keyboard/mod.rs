mod scancodes;
use scancodes::*;

use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use crate::arch::x86_64::IO_APIC;
use crate::arch::x86_64::io::{
    inb,
    outb,
};
use crate::kernel::acpi;
use crate::kernel::sync::Semaphore;
use crate::klog;
use crate::util::bitwise::{
    set_bit,
    unset_bit,
};

static KEYBOARD_GSI: AtomicUsize = AtomicUsize::new(1);
static EDGE: AtomicBool = AtomicBool::new(true);
static ACTIVE_HIGH: AtomicBool = AtomicBool::new(true);

const IDT_VECTOR: u8 = 33;

pub const KBD_BUFFER_SIZE: usize = 256;

static mut KBD_BUFFER: [u8; KBD_BUFFER_SIZE] = [0; KBD_BUFFER_SIZE];
static KBD_BUFFER_HEAD: AtomicUsize = AtomicUsize::new(0);
static KBD_BUFFER_TAIL: AtomicUsize = AtomicUsize::new(0);
static KBD_ITEMS_READY: Semaphore = Semaphore::new(0);

pub fn push_scancode(scancode: u8) {
    unsafe {
        let tail = KBD_BUFFER_TAIL.load(Ordering::Relaxed) % KBD_BUFFER_SIZE;
        KBD_BUFFER[tail] = scancode;
        KBD_BUFFER_TAIL.fetch_add(1, Ordering::Relaxed);
        KBD_ITEMS_READY.signal();
    }
}

fn check_madt_overrides() {
    let rsdp = acpi::rsdp::Rsdp::get();
    let sdt = acpi::sdt::SDTArray::get(rsdp.get_table());
    let madt = acpi::madt::parse_madt(&sdt);
    let iso = madt.overrides;
    for entry in iso {
        if entry.source == 1 {
            KEYBOARD_GSI.store(entry.gsi as usize, Ordering::Relaxed);
            if entry.flags & 0b11 == 3 {
                ACTIVE_HIGH.store(false, Ordering::Relaxed);
            }
            if entry.flags & 0b1100 == 11 {
                EDGE.store(false, Ordering::Relaxed);
            }
        }
    }
}

pub fn init_keyboard_irq() {
    check_madt_overrides();
    IO_APIC.lock().set_entry(
        KEYBOARD_GSI.load(Ordering::Relaxed) as u32,
        IDT_VECTOR,
        0,
        false,
        ACTIVE_HIGH.load(Ordering::Relaxed),
        EDGE.load(Ordering::Relaxed),
    );
    unsafe {
        outb(0x64, 0x20);
        let mut config = inb(0x60);
        config = set_bit(config, 0);
        config = unset_bit(config, 4);
        config = set_bit(config, 6); // translate set 2 to set 1
        outb(0x64, 0x60);
        outb(0x60, config);
    }
}

#[allow(unused)]
pub extern "C" fn kbd_processor_thread() -> ! {
    let mut shift_held = false;
    let mut caps_lock = false;
    let mut is_extended = false;

    loop {
        KBD_ITEMS_READY.wait();

        let scancode = unsafe {
            let head = KBD_BUFFER_HEAD.fetch_add(1, Ordering::Relaxed) % KBD_BUFFER_SIZE;
            KBD_BUFFER[head]
        };

        if scancode == 0xE0 {
            is_extended = true;
            continue;
        }

        let is_release = (scancode & 0x80) != 0;
        let key = (scancode & 0x7F) as usize;

        match key {
            0x2A | 0x36 => {
                shift_held = !is_release;
            }
            0x3A => {
                if !is_release {
                    caps_lock = !caps_lock;
                }
            }
            _ => {}
        }

        if !is_release {
            let mut c = { 
                if shift_held { KBD_US_SHIFT[key] } 
                else if is_extended { KBD_US_EXTENDED[key] }
                else { KBD_US_BASE[key] }
            };

            if caps_lock && c.is_ascii_alphabetic() {
                if c.is_ascii_lowercase() {
                    c = c.to_ascii_uppercase();
                } else {
                    c = c.to_ascii_lowercase();
                }
            }

            if c != '\0' {
                klog!("{}", c);
            }
        }

        is_extended = false;
    }
}
