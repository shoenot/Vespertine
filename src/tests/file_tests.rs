use crate::kernel::sync::RwLock;
use crate::{MODULE_REQUEST, klog, klogln};
use crate::kernel::object::invoke::{Invocation, InvocationError};
use crate::kernel::object::vfs::{kernel_register_obj, sys_invoke};
use crate::kernel::object::obj::KernelObject;
use crate::kernel::object::handle::{AccessRights, HandleID};
use crate::kernel::object::models::directory::*;
use crate::kernel::object::op::{DirectoryOp, FileOp};
use crate::kernel::object::models::file::*;

pub static ROOT_DIRECTORY: RwLock<Option<HandleID>> = RwLock::new(None);

use alloc::string::ToString;
use alloc::sync::Arc;
#[derive(Debug)]
pub struct TestDevice {}

impl KernelObject for TestDevice {
    fn invoke(&self, invocation: Invocation) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Ping => klogln!("Pong!"),
            _ => return Err(InvocationError::UnsupportedOperation),
        }
        Ok(0)
    }
}

pub fn init_vfs() {
    let root_dir = Arc::new(Directory::new());
    let dev_dir = Arc::new(Directory::new());
    let docs_dir = Arc::new(Directory::new());

    let root_handle = kernel_register_obj(root_dir, AccessRights::READ | AccessRights::WRITE);
    let dev_handle = kernel_register_obj(dev_dir, AccessRights::READ | AccessRights::WRITE);
    let docs_handle = kernel_register_obj(docs_dir, AccessRights::READ | AccessRights::WRITE);

    *ROOT_DIRECTORY.write() = Some(root_handle);

    klog!("Mounting /dev ... ");
    // mount '/dev' inside '/'
    let dev_name = "dev";
    sys_invoke(
        root_handle,
        Invocation::Directory(DirectoryOp::Link { name: dev_name.as_ptr(), name_len: dev_name.len(), handle_id: dev_handle }),
    )
    .expect("Failed to link /dev");
    klogln!("Mount success!");

    klog!("Mounting /docs ...");
    let docs_name = "docs";
    sys_invoke(
        root_handle,
        Invocation::Directory(DirectoryOp::Link { name: docs_name.as_ptr(), name_len: docs_name.len(), handle_id: docs_handle }),
    )
    .expect("Failed to link /docs");
    klogln!("Mount success!");
}

pub fn test_vfs_path_res(path: &str) -> Result<HandleID, InvocationError> {
    let mut current_handle = ROOT_DIRECTORY.read().expect("Root not initialized.");

    for component in path.split('/') {
        if component.is_empty() {
            continue;
        }

        let result = sys_invoke(
            current_handle,
            Invocation::Directory(DirectoryOp::Lookup { name: component.as_ptr(), name_len: component.len() }),
        );

        match result {
            Ok(next_handle_id) => {
                current_handle = HandleID(next_handle_id);
            }
            Err(e) => {
                klogln!("Path resolution failed at '{}': {:?}", component, e);
                return Err(e);
            }
        }
    }
    Ok(current_handle)
}

pub fn load_ramdisk_modules(root_handle: HandleID) {
    let response = match MODULE_REQUEST.response() {
        Some(resp) => resp,
        None => { klogln!("Nothing loaded by limine"); return; },
    };

    for module in response.modules() {
        let raw_path = module.path();
        let file_name = raw_path.split('/').last().unwrap_or(raw_path);

        let data_slice = module.data();

        let file_ptr = data_slice.as_ptr();
        let file_size = data_slice.len();

        klogln!("Found file '{}' at {:p} ({} bytes)", file_name, file_ptr, file_size);

        let file_obj = Arc::new(FileObj::new(file_ptr, file_size));
        let file_handle = kernel_register_obj(file_obj, AccessRights::READ);

        let invocation = Invocation::Directory(
            DirectoryOp::Link { 
                name: file_name.as_ptr(), 
                name_len: file_name.len(), 
                handle_id: file_handle 
            }
        );

        match sys_invoke(root_handle, invocation) {
            Ok(_) => klogln!("Successfully mounted /{}", file_name),
            Err(e) => klogln!("Failed to mount file: {:?}", e),
        }
    }
}

pub fn test_run() {
    let root_handle = test_vfs_path_res("/docs").expect("File not found (1)");
    load_ramdisk_modules(root_handle);
    let file_handle = test_vfs_path_res("/docs/filetest.txt").expect("File not found (2)");

    let mut read_buffer = [0u8; 128];
    let invocation = Invocation::File(FileOp::Read { offset: 0, buffer_ptr: read_buffer.as_mut_ptr(), len: read_buffer.len() });
    let bytes_read = sys_invoke(file_handle, invocation).expect("Failed to read file");
    if let Ok(txt) = str::from_utf8(&read_buffer[..bytes_read]) {
        klogln!("File contents: {}", txt);
    }
}

