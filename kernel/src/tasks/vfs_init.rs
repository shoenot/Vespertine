use alloc::sync::Arc;

use vespertine_abi::AccessRights;
use vespertine_abi::tag::{
    CAP_CLOCK,
    CAP_PORTAL_FACTORY,
    CAP_PROCMAN,
    CAP_SOCKFAC,
};

use crate::core::object::models::broker::Broker;
use crate::core::object::models::clock::Clock;
use crate::core::object::models::directory::*;
use crate::core::object::models::log::Log;
use crate::core::object::models::memman::MemoryManager;
use crate::core::object::models::mount::mount;
use crate::core::object::models::namespace::{
    DirLocation,
    kernel_namespace_authority,
    resolve_kernel_object,
};
use crate::core::object::models::portal::PortalFactory;
use crate::core::object::models::procman::ProcessManager;
use crate::core::object::models::socket::SocketFactory;
use crate::core::object::vfs::{
    ROOT_DIRECTORY,
    kernel_root_location,
    link_kernel_object,
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
    let block_root = mount_ext2_rootfs(blockdev.clone()).await;
    let root_location = kernel_root_location();
    let bootstrap_root = root_location.directory();

    mount(bootstrap_root, block_root).expect("[FATAL] Failed to mount disk root");

    klogln!("[SUCCESS] Disk root mounted at /");
    let root_dir = root_location.directory();

    let authority = kernel_namespace_authority();

    let sys_dir = resolve_kernel_object(&authority, root_location, "/System").await.expect("[FATAL] /System directory missing");

    let dev_dir = Arc::new(Directory::new());
    let srv_dir = Arc::new(Directory::new());

    // mount all dirs
    mount_kernel_object(root_dir, "Devices", dev_dir.clone()).await.expect("Failed to mount /Devices");
    mount_kernel_object(sys_dir.clone(), "Services", srv_dir.clone()).await.expect("Failed to mount /System/Services");

    let proc_man = Arc::new(ProcessManager {});
    let mut proc_man_broker = Broker::new();
    proc_man_broker.publish(CAP_PROCMAN, proc_man, AccessRights::READ | AccessRights::LIST | AccessRights::CREATE | AccessRights::EXECUTE);
    link_kernel_object(srv_dir.clone(), "ProcManager", Arc::new(proc_man_broker)).await.expect("Failed to mount ProcManager");

    let mem_man = Arc::new(MemoryManager {});
    link_kernel_object(srv_dir.clone(), "MemoryManager", mem_man).await.expect("Failed to mount MemoryManager");

    let clock = Arc::new(Clock {});
    let mut clock_broker = Broker::new();
    clock_broker.publish(CAP_CLOCK, clock, AccessRights::READ | AccessRights::WRITE);
    link_kernel_object(srv_dir.clone(), "Clock", Arc::new(clock_broker)).await.expect("Failed to mount Clock");

    let socket_fac = Arc::new(SocketFactory {});
    let mut sockfac_broker = Broker::new();
    sockfac_broker.publish(CAP_SOCKFAC, socket_fac, AccessRights::CREATE);
    link_kernel_object(srv_dir.clone(), "Socket", Arc::new(sockfac_broker)).await.expect("Failed to mount Socket");

    let mut portal_broker = Broker::new();
    portal_broker.publish(CAP_PORTAL_FACTORY, Arc::new(PortalFactory), AccessRights::CREATE);
    link_kernel_object(srv_dir.clone(), "Portal", Arc::new(portal_broker)).await.expect("Failed to mount Portal");

    let log_obj = Arc::new(Log::new());
    link_kernel_object(srv_dir, "Log", log_obj).await.expect("Failed to mount Log");

    let fb_obj = Arc::new(init_framebuffer());
    link_kernel_object(dev_dir, "Framebuffer", fb_obj).await.expect("Failed to mount Framebuffer");
}
