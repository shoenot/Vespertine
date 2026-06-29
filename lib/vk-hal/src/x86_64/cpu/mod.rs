use core::arch::asm;
pub mod cpuid;

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
