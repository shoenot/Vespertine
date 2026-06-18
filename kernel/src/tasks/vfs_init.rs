use alloc::sync::Arc;

use vespertine_abi::AccessRights;
use vespertine_abi::tag::{
    CAP_CLOCK,
    CAP_PROCMAN,
    CAP_SOCKFAC,
};

use crate::core::object::models::broker::Broker;
use crate::core::object::models::clock::Clock;
use crate::core::object::models::directory::*;
use crate::core::object::models::log::Log;
use crate::core::object::models::memman::MemoryManager;
use crate::core::object::models::mount_dir::MountDirectory;
use crate::core::object::models::namespace::{
    DirLocation,
    kernel_namespace_authority,
    resolve_kernel_object,
};
use crate::core::object::models::procman::ProcessManager;
use crate::core::object::models::socket::SocketFactory;
use crate::core::object::vfs::{
    ROOT_DIRECTORY,
    kernel_root_location,
    mount_kernel_object,
};
use crate::core::sync::KernelOnceCell;
use crate::drivers::video::init_framebuffer;
use crate::klogln;
use crate::storage::blockdev::AsyncBlockDevice;
use crate::storage::fs::mount_ext2_rootfs;

pub static BLOCK_DEVICE: KernelOnceCell<Arc<dyn AsyncBlockDevice>> = KernelOnceCell::new();

pub async fn init_vfs() {
    let blockdev = BLOCK_DEVICE.get().expect("[FATAL] No block device found for primary storage");
    let root = mount_ext2_rootfs(blockdev.clone()).await;
    let root_obj = ROOT_DIRECTORY.get().expect("[FATAL] ROOT_DIRECTORY uninitialized");
    let root_location = root_obj.as_any().downcast_ref::<DirLocation>().expect("ROOT_DIRECTORY is not a DirLocation");
    let root_dir = root_location.directory();
    let mount_dir = root_dir.as_any().downcast_ref::<MountDirectory>().expect("[FATAL] ROOT_DIRECTORY is not a MountDirectory");

    mount_dir.set_underlying(root);
    klogln!("[SUCCESS] Ext2 root directory mounted at /");

    let authority = kernel_namespace_authority();
    let root_location = kernel_root_location();

    let sys_obj = resolve_kernel_object(&authority, root_location, "/System").await.expect("System directory missing");
    let sys_mount = Arc::new(MountDirectory::new(sys_obj));
    mount_kernel_object(root_dir.clone(), "System", sys_mount.clone()).await.expect("Failed to mount /System");

    let dev_dir = Arc::new(Directory::new());
    let srv_dir = Arc::new(Directory::new());
    let log_dir = Arc::new(Directory::new());

    // mount all dirs
    mount_kernel_object(root_dir, "Devices", dev_dir.clone()).await.expect("Failed to mount /Devices");
    mount_kernel_object(sys_mount.clone(), "Services", srv_dir.clone()).await.expect("Failed to mount /System/Services");
    mount_kernel_object(sys_mount, "Logs", log_dir).await.expect("Failed to mount /System/Logs");

    let proc_man = Arc::new(ProcessManager {});
    let mut proc_man_broker = Broker::new();
    proc_man_broker.publish(CAP_PROCMAN, proc_man, AccessRights::CREATE | AccessRights::EXECUTE);
    mount_kernel_object(srv_dir.clone(), "ProcManager", Arc::new(proc_man_broker)).await.expect("Failed to mount ProcManager");

    let mem_man = Arc::new(MemoryManager {});
    mount_kernel_object(srv_dir.clone(), "MemoryManager", mem_man).await.expect("Failed to mount MemoryManager");

    let clock = Arc::new(Clock {});
    let mut clock_broker = Broker::new();
    clock_broker.publish(CAP_CLOCK, clock, AccessRights::READ | AccessRights::WRITE);
    mount_kernel_object(srv_dir.clone(), "Clock", Arc::new(clock_broker)).await.expect("Failed to mount Clock");

    let socket_fac = Arc::new(SocketFactory {});
    let mut sockfac_broker = Broker::new();
    sockfac_broker.publish(CAP_SOCKFAC, socket_fac, AccessRights::CREATE);
    mount_kernel_object(srv_dir.clone(), "Socket", Arc::new(sockfac_broker)).await.expect("Failed to mount Socket");

    let log_obj = Arc::new(Log {});
    mount_kernel_object(srv_dir, "Log", log_obj).await.expect("Failed to mount Log");

    let fb_obj = Arc::new(init_framebuffer());
    mount_kernel_object(dev_dir, "Framebuffer", fb_obj).await.expect("Failed to mount Framebuffer");
}
