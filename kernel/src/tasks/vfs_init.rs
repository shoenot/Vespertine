use alloc::slice;
use alloc::sync::Arc;

use vespertine_abi::tag::{CAP_CLOCK, CAP_PROCMAN, CAP_SOCKFAC};
use vespertine_abi::{
    AccessRights,
    HandleID,
};

use crate::core::object::models::broker::Broker;
use crate::core::object::models::clock::Clock;
use crate::core::object::models::directory::*;
use crate::core::object::models::log::Log;
use crate::core::object::models::memman::MemoryManager;
use crate::core::object::models::mount_dir::MountDirectory;
use crate::core::object::models::namespace::{DirLocation, kernel_namespace_authority, resolve_kernel_object};
use crate::core::object::models::procman::ProcessManager;
use crate::core::object::models::socket::SocketFactory;
use crate::core::object::vfs::{
    ROOT_DIRECTORY, kernel_register_obj, kernel_root_location, mount_kernel_dir
};
use crate::core::sync::KernelOnceCell;
use crate::core::thread::get_current_process;
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
    let sys_mount_handle = kernel_register_obj(sys_mount, AccessRights::all());
    mount_kernel_dir("System", sys_mount_handle, HandleID(0)).await;

    let dev_dir = Arc::new(Directory::new());
    let srv_dir = Arc::new(Directory::new());
    let log_dir = Arc::new(Directory::new());

    let dev_handle = kernel_register_obj(dev_dir, AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE);
    let srv_handle = kernel_register_obj(srv_dir, AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE);
    let log_handle = kernel_register_obj(log_dir, AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE);

    // mount all dirs
    mount_kernel_dir("Devices", dev_handle, HandleID(0)).await;
    mount_kernel_dir("Services", srv_handle, sys_mount_handle).await;
    mount_kernel_dir("Logs", log_handle, sys_mount_handle).await;

    let proc_man = Arc::new(ProcessManager {});
    let mut proc_man_broker = Broker::new();
    proc_man_broker.publish(CAP_PROCMAN, proc_man, AccessRights::CREATE | AccessRights::EXECUTE);
    let proc_man_broker_handle = kernel_register_obj(Arc::new(proc_man_broker), AccessRights::all());
    mount_kernel_dir("ProcManager", proc_man_broker_handle, srv_handle).await;

    let mem_man = Arc::new(MemoryManager {});
    let mem_man_handle = kernel_register_obj(mem_man, AccessRights::all());
    mount_kernel_dir("MemoryManager", mem_man_handle, srv_handle).await;

    let clock = Arc::new(Clock {});
    let mut clock_broker = Broker::new();
    clock_broker.publish(CAP_CLOCK, clock, AccessRights::READ | AccessRights::WRITE);
    let clock_broker_handle = kernel_register_obj(Arc::new(clock_broker), AccessRights::READ);
    mount_kernel_dir("Clock", clock_broker_handle, srv_handle).await;

    let socket_fac = Arc::new(SocketFactory {});
    let mut sockfac_broker = Broker::new();
    sockfac_broker.publish(CAP_SOCKFAC, socket_fac, AccessRights::CREATE);
    let socket_broker_handle = kernel_register_obj(Arc::new(sockfac_broker), AccessRights::READ);
    mount_kernel_dir("Socket", socket_broker_handle, srv_handle).await;

    let log_obj = Arc::new(Log {});
    let log_handle = kernel_register_obj(log_obj, AccessRights::WRITE);
    mount_kernel_dir("Log", log_handle, srv_handle).await;

    let fb_obj = Arc::new(init_framebuffer());
    let fb_handle = kernel_register_obj(fb_obj, AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);
    mount_kernel_dir("Framebuffer", fb_handle, dev_handle).await;
}
