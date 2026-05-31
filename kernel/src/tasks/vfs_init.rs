use alloc::slice;
use alloc::sync::Arc;

use vespertine_abi::{
    AccessRights,
    HandleID,
};

use crate::core::object::models::clock::Clock;
use crate::core::object::models::directory::*;
use crate::core::object::models::memman::MemoryManager;
use crate::core::object::models::procman::ProcessManager;
use crate::core::object::models::socket::SocketFactory;
use crate::core::object::vfs::{
    kernel_register_obj,
    mount_kernel_dir,
};
use crate::core::sync::KernelOnceCell;
use crate::storage::blockdev::AsyncBlockDevice;
use crate::storage::fs::mount_ext2_rootfs;
use crate::drivers::tar::{
    get_ramdisk_ptr,
    get_ramdisk_size,
    parse_tar,
};
use crate::drivers::video::init_framebuffer;
use crate::klogln;

pub static BLOCK_DEVICE: KernelOnceCell<Arc<dyn AsyncBlockDevice>> = KernelOnceCell::new();

pub async fn init_vfs() {
    let dev_dir = Arc::new(Directory::new());
    let sys_dir = Arc::new(Directory::new());
    let srv_dir = Arc::new(Directory::new());
    let log_dir = Arc::new(Directory::new());

    let dev_handle = kernel_register_obj(dev_dir, AccessRights::READ | AccessRights::WRITE);
    let sys_handle = kernel_register_obj(sys_dir, AccessRights::READ | AccessRights::WRITE);
    let srv_handle = kernel_register_obj(srv_dir, AccessRights::READ | AccessRights::WRITE);
    let log_handle = kernel_register_obj(log_dir, AccessRights::READ | AccessRights::WRITE);

    // mount all dirs
    mount_kernel_dir("Devices", dev_handle, HandleID(0)).await;
    mount_kernel_dir("System", sys_handle, HandleID(0)).await;
    mount_kernel_dir("Services", srv_handle, sys_handle).await;
    mount_kernel_dir("Logs", log_handle, sys_handle).await;

    let ptr = get_ramdisk_ptr();
    let size = get_ramdisk_size();
    let ramdisk_slice = unsafe { slice::from_raw_parts(ptr, size) };
    parse_tar(ramdisk_slice).await.expect("Failed to parse ramdisk");

    let proc_man = Arc::new(ProcessManager {});
    let proc_man_handle = kernel_register_obj(proc_man, AccessRights::all());
    mount_kernel_dir("ProcessManager", proc_man_handle, srv_handle).await;

    let mem_man = Arc::new(MemoryManager {});
    let mem_man_handle = kernel_register_obj(mem_man, AccessRights::all());
    mount_kernel_dir("MemoryManager", mem_man_handle, srv_handle).await;

    let clock = Arc::new(Clock {});
    let clock_handle = kernel_register_obj(clock, AccessRights::all());
    mount_kernel_dir("Clock", clock_handle, srv_handle).await;

    let socket_fac = Arc::new(SocketFactory {});
    let socket_fac_handle = kernel_register_obj(socket_fac, AccessRights::all());
    mount_kernel_dir("SocketFactory", socket_fac_handle, srv_handle).await;

    let fb_obj = Arc::new(init_framebuffer());
    let fb_handle = kernel_register_obj(fb_obj, AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);
    mount_kernel_dir("Framebuffer", fb_handle, dev_handle).await;

    if let Some(blockdev) = BLOCK_DEVICE.get() {
        let root = mount_ext2_rootfs(blockdev.clone()).await;
        let handle = kernel_register_obj(root, AccessRights::READ | AccessRights::WRITE);
        mount_kernel_dir("Disk", handle, HandleID(0)).await;
        klogln!("[SUCCESS] Ext2 filesystem mounted at /Disk");
    }
}
