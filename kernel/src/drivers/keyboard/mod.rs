pub mod scancodes;
mod keymaps;
use core::num;
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use scancodes::*;
use vespertine_abi::key::{KeyCode, KeyEvent, KeyState, Modifiers};
use vespertine_abi::op::FileOp;
use vespertine_abi::{
    HandleID,
    Invocation,
};

use hal::io::{
    inb,
    outb,
};
use crate::executor::syscall_bridge::handle_sys_invoke;
use crate::sync::Semaphore;
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

pub fn init_keyboard_irq() {
    hal::interrupts::route_isa_irq(1, IDT_VECTOR, 0);
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
    let mut super_held = false;
    let mut caps_lock = false;
    let mut num_lock = true;    // default on
    let mut scroll_lock = false;
    let mut is_extended = false;

    loop {
        KBD_ITEMS_READY.wait();
        let mut events = [KeyEvent::zeroed(); 64];
        let mut event_count = 0;

        loop {
            let scancode = pop_scancode();

            if scancode == 0xE0 {
                is_extended = true;
            } else {
                let is_release = (scancode & 0x80) != 0;
                let key = (scancode & 0x7F) as usize;

                let code = scancode_to_keycode(key, is_extended);

                match code {
                    KeyCode::LeftShift | KeyCode::RightShift    => shift_held = !is_release,
                    KeyCode::LeftCtrl | KeyCode::RightCtrl      => ctrl_held = !is_release,
                    KeyCode::LeftAlt | KeyCode::RightAlt        => alt_held = !is_release,
                    KeyCode::LeftSuper | KeyCode::RightSuper    => super_held = !is_release,
                    KeyCode::CapsLock if !is_release            => caps_lock = !caps_lock,
                    KeyCode::NumLock if !is_release             => num_lock = !num_lock,
                    KeyCode::ScrollLock if !is_release          => scroll_lock = !scroll_lock,
                    _ => {},
                }

                let state = if is_release {
                    KeyState::Released
                } else {
                    KeyState::Pressed
                };

                let mut mods = Modifiers::new();
                if shift_held       { mods = mods.insert(Modifiers::SHIFT); }
                if ctrl_held        { mods = mods.insert(Modifiers::CTRL); }
                if alt_held         { mods = mods.insert(Modifiers::ALT); }
                if super_held       { mods = mods.insert(Modifiers::SUPER); }
                if caps_lock        { mods = mods.insert(Modifiers::CAPSL); }
                if num_lock         { mods = mods.insert(Modifiers::NUML); }
                if scroll_lock      { mods = mods.insert(Modifiers::SCRL); }

                let unicode = if !is_release {
                    active_keymap().map_key(code, mods)
                } else {
                    '\0'
                };

                if code != KeyCode::Unknown && event_count < events.len() {
                    events[event_count] = KeyEvent {
                        code, state, mods, unicode,
                    };
                    event_count += 1;
                }

                is_extended = false;
            }

            if !KBD_ITEMS_READY.try_wait() {
                break;
            }
        }

        if event_count > 0 {
            let byte_len = event_count * core::mem::size_of::<KeyEvent>();
            let write_op = Invocation::File(FileOp::Write { offset: 0, buffer_ptr: events.as_ptr() as usize, len: byte_len });
            let _ = handle_sys_invoke(chan_handle, write_op);
        }
    }
}
