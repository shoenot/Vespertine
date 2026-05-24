use core::panic::PanicInfo;

use crate::drivers::logger::LOGGER;
use crate::klogln;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { LOGGER.force_unlock() };
    klogln!("!------------- KERNEL PANIC -------------!");
    klogln!("{}\n", info);
    crate::arch::hcf();
}
