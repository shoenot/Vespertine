#![no_std]
#![no_main]
extern crate alloc;
mod arch;
mod boot;
mod drivers;
mod core;
mod memory;
mod panic;
mod tasks;
mod tests;
mod util;
mod syscall;

use ::core::ptr::read_volatile;
use ::core::sync::atomic::Ordering;

use crate::core::asynchronous::Executor;
use crate::core::cpu::init_smp;
use crate::core::time;
use crate::drivers::pci::{PCI_DEVICES, enumerate_pci_devices};
use crate::drivers::virtio::blk::{VirtioBlockDevice, init_block_device, virtio_blk_poll_thread};
use crate::drivers::virtio::mmio::init_virtio;
use alloc::sync::Arc;
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
    BlockSize,
    BOOTSTRAP_ALLOC,
};

use crate::arch::x86_64::cpu::core::{init_timer_daemon, CPULocalData};
use crate::core::object::handle::{AccessRights, HandleTable};
use vespertine_abi::HandleID;
use crate::core::object::models::directory::Directory;
use crate::core::object::models::process::{Process, ProcessControlBlock};
use crate::core::object::vfs::ROOT_DIRECTORY;
use crate::core::sync::KernelOnceCell;
use crate::core::thread::dispatch::spawn_kernel_thread;
use crate::core::thread::priority::ThreadPriority;
use crate::core::time::datetime::epoch_to_datetime;
use crate::drivers::keyboard::init_keyboard_irq;
use crate::memory::{ALLOCATOR, GLOBAL_PMM, HHDMOFFSET};

pub static KERNEL_PROCESS: KernelOnceCell<Process> = KernelOnceCell::new();

pub fn init_kernel_process() {
    KERNEL_PROCESS.get_or_init(|| {
        let proc = ProcessControlBlock::new(HandleTable::new());
        let root = ROOT_DIRECTORY.get_or_init(|| Arc::new(Directory::new())).clone();
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

    let mut blk = init_block_device().unwrap();

    let blk_ptr = &mut blk as *mut VirtioBlockDevice as usize;
    spawn_kernel_thread(
        virtio_blk_poll_thread as *const () as usize,
        blk_ptr, 
        ThreadPriority::HIGH, 
        KERNEL_PROCESS.clone(),
    );

    let executor = Executor::new();

    executor.spawn(async move {
        klogln!("[INFO] Async read verification task started");

        let buf_phys = ALLOCATOR.alloc(BlockSize::Normal);
        let buf_virt = buf_phys + *HHDMOFFSET;

        let blk_dev = blk_ptr as *mut VirtioBlockDevice;
        match unsafe { (*blk_dev).read_sectors_async(0, 1, buf_phys as u64) } {
            Ok(future) => {
                klogln!("[INFO] Waiting for read future...");
                if future.await.is_ok() {
                    klogln!("[SUCCESS] Async read success. Sector 0 data:");
                    let mut data = [0u8; 16];
                    for i in 0..16 {
                        let addr = buf_virt + i;
                        let byte = unsafe {
                            read_volatile(addr as *const u8)
                        };
                        data[i] = byte;
                    }
                    klogln!("{}", str::from_utf8(&data).expect(""));
                } else {
                    klogln!("[ERROR] Async read failed");
                }
            },
            Err(_) => {
                klogln!("[ERROR] Async read failed");
            }
        }
    }); 


    time::init_realtime();
    klogln!("[SUCCESS] Initialized Real Time Clock.");
    klogln!("[INFO] Current date and time: {}", epoch_to_datetime(time::get_realtime()));

    init_keyboard_irq();
    enable_interrupts();

    spawn_kernel_thread(tasks::initializer as *const () as usize, 0, ThreadPriority::MAXIMUM, KERNEL_PROCESS.clone());

    terminate_thread!();
}
