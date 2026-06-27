use core::arch::asm;

#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("hlt", options(nomem, nostack));
    }
}

pub fn halt_loop() -> ! {
    loop {
        halt();
    }
}
