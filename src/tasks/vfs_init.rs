use crate::drivers::tar::{get_ramdisk_ptr, get_ramdisk_size, parse_tar};
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
}



//
// // pub fn test_run() {
// //     let root_handle = test_vfs_path_res("/docs").expect("File not found (1)");
// //     load_ramdisk_modules(root_handle);
// //     let file_handle = test_vfs_path_res("/docs/filetest.txt").expect("File not found (2)");
// //
// //     let mut read_buffer = [0u8; 128];
// //     let invocation = Invocation::File(FileOp::Read { offset: 0, buffer_ptr: read_buffer.as_mut_ptr(), len: read_buffer.len() });
// //     let bytes_read = kernel_invoke(file_handle, invocation).expect("Failed to read file");
// //     if let Ok(txt) = str::from_utf8(&read_buffer[..bytes_read]) {
// //         klogln!("File contents: {}", txt);
// //     }
// // }
// //
