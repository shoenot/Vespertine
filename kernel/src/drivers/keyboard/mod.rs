mod scancodes;
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use scancodes::*;
use vespertine_abi::op::FileOp;
use vespertine_abi::{
    HandleID,
    Invocation,
};

use crate::arch::x86_64::IO_APIC;
use crate::arch::x86_64::io::{
    inb,
    outb,
};
use crate::core::acpi;
use crate::core::asynchronous::syscall_bridge::handle_sys_invoke;
use crate::core::sync::Semaphore;
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
        let head = KBD_BUFFER_HEAD.load(Ordering::Acquire);
        let tail = KBD_BUFFER_TAIL.load(Ordering::Relaxed);
        if tail.wrapping_sub(head) >= KBD_BUFFER_SIZE {
            return;
        }

        KBD_BUFFER[tail % KBD_BUFFER_SIZE] = scancode;
        KBD_BUFFER_TAIL.store(tail.wrapping_add(1), Ordering::Release);
        KBD_ITEMS_READY.signal();
    }
}

fn pop_scancode() -> u8 {
    unsafe {
        let head = KBD_BUFFER_HEAD.load(Ordering::Relaxed);
        let scancode = KBD_BUFFER[head % KBD_BUFFER_SIZE];
        KBD_BUFFER_HEAD.store(head.wrapping_add(1), Ordering::Release);
        scancode
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

pub extern "C" fn kbd_processor_thread(chan_handle_id: usize) -> ! {
    let chan_handle = HandleID(chan_handle_id);
    let mut shift_held = false;
    let mut ctrl_held = false;
    let mut alt_held = false;
    let mut caps_lock = false;
    let mut is_extended = false;

    loop {
        KBD_ITEMS_READY.wait();
        let mut output = [0u8; KBD_BUFFER_SIZE * 2];
        let mut output_len = 0;

        loop {
            let scancode = pop_scancode();

            if scancode == 0xE0 {
                is_extended = true;
            } else {
                let is_release = (scancode & 0x80) != 0;
                let key = (scancode & 0x7F) as usize;

                match key {
                    0x1D => ctrl_held = !is_release,
                    0x38 => alt_held = !is_release,
                    0x2A | 0x36 => shift_held = !is_release,
                    0x3A if !is_release => caps_lock = !caps_lock,
                    _ => {}
                }
                let mut handled_sequence = false;
                if !is_release && !matches!(key, 0x1D | 0x2A | 0x36 | 0x38 | 0x3A) {
                    if is_extended {
                        let sequence: &[u8] = match key {
                            0x48 => b"\x1b[A", // Up
                            0x50 => b"\x1b[B", // Down
                            0x4D => b"\x1b[C", // Right
                            0x4B => b"\x1b[D", // Left
                            _ => &[],
                        };

                        if !sequence.is_empty() {
                            if output_len + sequence.len() <= output.len() {
                                output[output_len..output_len + sequence.len()]
                                    .copy_from_slice(sequence);
                                output_len += sequence.len();
                            }
                            handled_sequence = true;
                        }
                    }

                    if !handled_sequence {
                        let mut c = if shift_held {
                            KBD_US_SHIFT[key]
                        } else if is_extended {
                            KBD_US_EXTENDED[key]
                        } else {
                            KBD_US_BASE[key]
                        };

                        if caps_lock && c.is_ascii_alphabetic() {
                            c = if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
                        }

                        if ctrl_held {
                            c = match c {
                                '@' | ' ' => '\x00',
                                'a'..='z' => ((c as u8 - b'a') + 1) as char,
                                'A'..='Z' => ((c as u8 - b'A') + 1) as char,
                                '[' => '\x1b',
                                '\\' => '\x1c',
                                ']' => '\x1d',
                                '^' => '\x1e',
                                '_' => '\x1f',
                                '?' => '\x7f',
                                _ => c,
                            };
                        }

                        if alt_held && output_len < output.len() {
                            output[output_len] = 0x1b;
                            output_len += 1;
                        }

                        if c != '\0' {
                            let mut byte_buffer = [0u8; 4];
                            let bytes = c.encode_utf8(&mut byte_buffer).as_bytes();
                            if output_len + bytes.len() <= output.len() {
                                output[output_len..output_len + bytes.len()].copy_from_slice(bytes);
                                output_len += bytes.len();
                            }
                        }
                    }
                }

                is_extended = false;
            }

            if !KBD_ITEMS_READY.try_wait() {
                break;
            }
        }

        if output_len > 0 {
            let write_op = Invocation::File(FileOp::Write { offset: 0, buffer_ptr: output.as_ptr() as usize, len: output_len });
            let _ = handle_sys_invoke(chan_handle, write_op);
        }
    }
}
