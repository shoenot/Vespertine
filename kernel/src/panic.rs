use core::panic::PanicInfo;

use hal::cpu::halt_loop;

use crate::drivers::logger::LOGGER;
use crate::klogln;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { LOGGER.force_unlock() };
    klogln!("!------------- KERNEL PANIC -------------!");
    klogln!("{}\n", info);
    halt_loop();
}
