use crate::drivers::tar::{get_ramdisk_ptr, get_ramdisk_size, parse_tar};
use crate::kernel::object::models::clock::Clock;
use crate::kernel::object::models::console::ConsoleWriter;
use crate::kernel::object::models::memman::MemoryManager;
use crate::kernel::object::models::procman::ProcessManager;
use crate::{MODULE_REQUEST, klog, klogln};
use crate::kernel::object::invoke::{Invocation, InvocationError};
use crate::kernel::object::vfs::{kernel_register_obj, mount_kernel_dir, kernel_invoke};
use crate::kernel::object::obj::KernelObject;
use crate::kernel::object::handle::{AccessRights, HandleID};
use crate::kernel::object::models::directory::*;

use alloc::sync::Arc;
#[derive(Debug)]
pub struct TestDevice {}

impl KernelObject for TestDevice {
    fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Ping => klogln!("Pong!"),
            _ => return Err(InvocationError::UnsupportedOperation),
        }
        Ok(0)
    }
}

pub fn init_vfs() {
    let dev_dir = Arc::new(Directory::new());
    let obj_dir = Arc::new(Directory::new());
    let chan_dir = Arc::new(Directory::new());

    let dev_handle = kernel_register_obj(dev_dir, AccessRights::READ | AccessRights::WRITE);
    let obj_handle = kernel_register_obj(obj_dir, AccessRights::READ | AccessRights::WRITE);
    let chan_handle = kernel_register_obj(chan_dir, AccessRights::READ | AccessRights::WRITE);

    // mount all dirs 
    mount_kernel_dir("Devices", dev_handle, HandleID(0));
    mount_kernel_dir("Objects", obj_handle, HandleID(0));
    mount_kernel_dir("Channels", chan_handle, obj_handle);

    let ptr = get_ramdisk_ptr();
    let size = get_ramdisk_size();
    parse_tar(ptr, size).expect("Failed to parse ramdisk");

    let proc_man = Arc::new(ProcessManager {});
    let proc_man_handle = kernel_register_obj(proc_man, AccessRights::all());
    mount_kernel_dir("ProcessManager", proc_man_handle, obj_handle);

    let mem_man = Arc::new(MemoryManager {});
    let mem_man_handle = kernel_register_obj(mem_man, AccessRights::all());
    mount_kernel_dir("MemoryManager", mem_man_handle, obj_handle);

    let clock = Arc::new(Clock {});
    let clock_handle = kernel_register_obj(clock, AccessRights::all());
    mount_kernel_dir("Clock", clock_handle, obj_handle);

    let console = Arc::new(ConsoleWriter {});
    let console_handle = kernel_register_obj(console, AccessRights::all());
    mount_kernel_dir("ConsoleWriter", console_handle, obj_handle);
}
