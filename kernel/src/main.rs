#![no_std]
#![no_main]
extern crate alloc;
mod cpu;
mod drivers;
mod executor;
mod interrupts;
mod process;
mod memory;
mod object;
mod panic;
mod program;
mod storage;
mod sched;
mod security;
mod syscall;
mod sync;
mod init;
mod time;
#[cfg(feature = "diagnostics")]
mod tests;
mod util;

use alloc::sync::Arc;
use hal::platform::PlatformInit;

use ::core::sync::atomic::Ordering;
use crate::cpu::{KernelCoreData, current_core_mut, hal_boot_alloc, init_bootstrap_core};
use crate::process::KERNEL_PROCESS;
use crate::time::callout::init_timer_daemon;
use cpu::ap_entry::BSP_CR3;
use drivers::logger::LOGGER;
use hal::interrupts::enable_interrupts;
use hal::mmu::get_cr3;
use memory::{
    BOOTSTRAP_ALLOC,
    PageSize,
};
pub use vespertine_common::define_bitflags;

use crate::cpu::init_smp;
use crate::sched::dispatch::spawn_kernel_thread;
use crate::sched::priority::ThreadPriority;
use crate::drivers::keyboard::init_keyboard_irq;
use crate::drivers::pci::enumerate_pci_devices;
use crate::drivers::virtio::blk::init_block_device;
use crate::drivers::virtio::mmio::init_virtio;
use crate::memory::{
    GLOBAL_PMM, PAGER, hal_map_mmio
};
use crate::storage::blockdev::AsyncBlockDevice;
use crate::init::vfs_init::BLOCK_DEVICE;

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    LOGGER.lock().init();

    let boot_mode = if cfg!(feature = "diagnostics") { "diagnostic boot" } else { "normal boot" };
    klogln!(Info, "Boot", "Vespertine {} ({boot_mode})", env!("CARGO_PKG_VERSION"));
    memory::init();
    let bootstrap_page = GLOBAL_PMM.lock().alloc(PageSize::Size2M).unwrap() as usize;
    BOOTSTRAP_ALLOC.lock().init(bootstrap_page);

    interrupts::init();
    let platform_hooks = PlatformInit { map_mmio: hal_map_mmio };
    hal::platform::init_early(platform_hooks);
    init_bootstrap_core();
    hal::platform::init();

    hal::cpu::init_bsp_state(hal_boot_alloc);

    hal::interrupts::init_platform_interrupts();

    process::init_kernel_process();

    current_core_mut().scheduler.init_threads(0);

    time::init();
    let data_ptr = current_core_mut() as *mut KernelCoreData;
    init_timer_daemon(data_ptr);

    let cr3 = get_cr3();
    BSP_CR3.store(cr3, Ordering::Release);

    init_smp();
    #[cfg(feature = "diagnostics")]
    tests::shootdown_tests::run_bsp();

    enumerate_pci_devices();

    init_virtio();

    let blk = init_block_device().expect("No usable VirtIO block device");
    let blk_arc = Arc::new(blk);

    let _ = blk_arc.setup_interrupts();

    {
        let kernel_root_phys = PAGER.lock().root_phys().0;
        KERNEL_PROCESS.vmm.write().as_mut().expect("kernel address space missing")
            .refresh_kernel_mappings(kernel_root_phys as usize);
    }

    let blk_dyn: Arc<dyn AsyncBlockDevice> = blk_arc.clone();
    BLOCK_DEVICE.get_or_init(|| blk_dyn);

    time::init_realtime();

    init_keyboard_irq();
    enable_interrupts();

    spawn_kernel_thread(init::initializer as *const () as usize, 0, ThreadPriority::MAXIMUM, KERNEL_PROCESS.clone());

    terminate_thread!();
}
