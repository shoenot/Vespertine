#![no_std]
#![no_main]
extern crate alloc;
mod arch;
mod boot;
mod core;
mod drivers;
mod interrupts;
mod memory;
mod panic;
mod storage;
mod syscall;
mod tasks;
mod tests;
mod util;

use alloc::sync::Arc;

use ::core::sync::atomic::Ordering;
use arch::x86_64::hcf;
use arch::{
    enable_interrupts,
    get_core_data,
};
use boot::smp::BSP_CR3;
pub use boot::*;
use drivers::logger::LOGGER;
use memory::paging::get_cr3;
use memory::{
    BOOTSTRAP_ALLOC,
    BlockSize,
};
use vespertine_abi::HandleID;

use crate::arch::x86_64::cpu::core::{
    CPULocalData,
    init_timer_daemon,
};
use crate::core::cpu::init_smp;
use crate::core::object::handle::{
    AccessRights,
    HandleTable,
};
use crate::core::object::models::directory::Directory;
use crate::core::object::models::mount_dir::MountDirectory;
use crate::core::object::models::process::{
    Process,
    ProcessControlBlock,
};
use crate::core::object::vfs::ROOT_DIRECTORY;
use crate::core::sync::KernelOnceCell;
use crate::core::thread::dispatch::spawn_kernel_thread;
use crate::core::thread::priority::ThreadPriority;
use crate::core::time;
use crate::core::time::datetime::epoch_to_datetime;
use crate::drivers::keyboard::init_keyboard_irq;
use crate::drivers::pci::{
    PCI_DEVICES,
    enumerate_pci_devices,
};
use crate::drivers::virtio::blk::init_block_device;
use crate::drivers::virtio::mmio::init_virtio;
use crate::memory::GLOBAL_PMM;
use crate::storage::blockdev::AsyncBlockDevice;
use crate::tasks::vfs_init::BLOCK_DEVICE;

pub static KERNEL_PROCESS: KernelOnceCell<Process> = KernelOnceCell::new();

pub fn init_kernel_process() {
    KERNEL_PROCESS.get_or_init(|| {
        let mut proc = ProcessControlBlock::new(HandleTable::new());
        if let Some(p) = Arc::get_mut(&mut proc) {
            p.pml4_addr = get_cr3() as usize & 0x000F_FFFF_FFFF_F000;
        }
        let root = ROOT_DIRECTORY
            .get_or_init(|| {
                let root_mem = Arc::new(Directory::new());
                Arc::new(MountDirectory::new(root_mem))
            })
            .clone();
        proc.proc_handles.write().insert_at(HandleID(0), root, AccessRights::all());
        proc.proc_handles.write().insert_at(HandleID(1), proc.clone(), AccessRights::all());
        proc
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    LOGGER.lock().init();

    memory::init();
    let bootstrap_page = GLOBAL_PMM.lock().alloc(BlockSize::Huge).unwrap() as usize;
    BOOTSTRAP_ALLOC.lock().init(bootstrap_page);

    arch::init();
    arch::init_bootstrap_core();

    klogln!("[INFO] GS Base initialized. Starting FPU...");
    arch::init_fpu(true);

    klogln!("[INFO] FPU initialized. Starting Global APICs...");
    arch::init_global_apics();

    init_kernel_process();

    get_core_data().scheduler.init_threads(0);

    time::init();
    let data_ptr = get_core_data() as *mut CPULocalData;
    init_timer_daemon(data_ptr);

    let cr3 = get_cr3();
    BSP_CR3.store(cr3, Ordering::Release);

    init_smp();

    enumerate_pci_devices();
    for dev in &*PCI_DEVICES.lock() {
        klogln!("{}", dev);
    }

    init_virtio();

    let blk = init_block_device().expect("Failed to init block device");
    let blk_arc = Arc::new(blk);

    // setup_interrupts now handles spawning per-core worker threads and MSI-X steering
    blk_arc.setup_interrupts().ok();

    let blk_dyn: Arc<dyn AsyncBlockDevice> = blk_arc.clone();
    BLOCK_DEVICE.get_or_init(|| blk_dyn);

    time::init_realtime();
    klogln!("[SUCCESS] Initialized Real Time Clock.");
    klogln!("[INFO] Current date and time: {}", epoch_to_datetime(time::get_realtime().0));

    init_keyboard_irq();
    enable_interrupts();

    spawn_kernel_thread(tasks::initializer as *const () as usize, 0, ThreadPriority::MAXIMUM, KERNEL_PROCESS.clone());

    terminate_thread!();
}
