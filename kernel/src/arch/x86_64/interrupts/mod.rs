pub(crate) mod handle;
pub(crate) mod idt;
pub(crate) mod shootdown;
pub(crate) mod extable;

use core::arch::asm;

#[inline]
pub(crate) fn disable_interrupts() {
    unsafe {
        asm!("cli", options(nostack));
    }
}

#[inline]
pub(crate) fn enable_interrupts() {
    unsafe {
        asm!("sti", options(nostack));
    }
}

#[inline]
pub(crate) fn interrupts_enabled() -> bool {
    let rflags: usize;
    unsafe {
        asm!("pushf",
            "pop {}",
            out(reg) rflags,
            options(nomem, preserves_flags))
    }
    (rflags & (1 << 9)) != 0
}
