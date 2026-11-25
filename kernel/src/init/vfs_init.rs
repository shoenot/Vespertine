use alloc::sync::Arc;

use vespertine_abi::{AccessRights, CapabilityID, HandleID};
use vespertine_abi::tag::{
    CAP_CLOCK, CAP_LOGGER, CAP_PORTAL_FACTORY, CAP_PROCMAN, CAP_SOCKFAC,
};

use crate::object::broker::Broker;
use crate::object::clock::Clock;
use crate::object::fs::directory::*;
use crate::object::invoke::InvocationError;
use crate::object::memman::MemoryManager;
use crate::object::mount::mount;
use crate::object::namespace::{
    kernel_namespace_authority,
    resolve_kernel_object,
};
use crate::object::ipc::portal::PortalFactory;
use crate::object::obj::KernelObject;
use crate::process::procman::ProcessManager;
use crate::object::ipc::socket::SocketFactory;
use crate::object::vfs::{
    kernel_register_obj, kernel_root_location, link_kernel_object, mount_kernel_object,
};
use crate::sync::KernelOnceCell;
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

    klogln!(Info, "Filesystem", "ext2 root mounted at /");
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

    let fb_obj = Arc::new(init_framebuffer());
    link_kernel_object(dev_dir, "Framebuffer", fb_obj).await.expect("Failed to mount Framebuffer");
}


pub async fn init_service(
    service: Arc<dyn KernelObject>, 
    name: &'static str, 
    capability: CapabilityID,
    public_rights: AccessRights,
    private_rights: AccessRights,
) -> Result<HandleID, InvocationError> {
    let authority = kernel_namespace_authority();
    let services = resolve_kernel_object(&authority, kernel_root_location(), "/System/Services").await?;

    let mut service_broker = Broker::new();
    service_broker.publish(capability, service.clone(), public_rights);
    link_kernel_object(services, name, Arc::new(service_broker)).await?;

    Ok(kernel_register_obj(service, private_rights)) 
}

pub async fn init_device(device: Arc<dyn KernelObject>, name: &'static str, rights: AccessRights) -> Result<HandleID, InvocationError> {
    let authority = kernel_namespace_authority();
    let devices = resolve_kernel_object(&authority, kernel_root_location(), "/Devices").await?;

    link_kernel_object(devices, name, device.clone()).await?;

    Ok(kernel_register_obj(device, rights))
}
