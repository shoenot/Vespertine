use alloc::sync::Arc;

use crate::kernel::object::handle::{
    AccessRights,
    HandleID,
};
use crate::kernel::object::invoke::{
    Invocation,
    InvocationError,
};
use crate::kernel::object::obj::{
    KernelHandleTable,
    KernelObject,
};
use crate::kernel::object::op::DirectoryOp;
use crate::kernel::sync::RwLock;
use crate::{
    klog, klogln
};

pub static PRINCIPAL_HANDLE_TABLE: RwLock<KernelHandleTable> = RwLock::new(KernelHandleTable::new());
pub static ROOT_DIRECTORY: RwLock<Option<HandleID>> = RwLock::new(None);

pub fn kernel_register_obj(obj: Arc<dyn KernelObject>, init_rights: AccessRights) -> HandleID {
    let mut table = PRINCIPAL_HANDLE_TABLE.write();
    table.insert(obj, init_rights)
}

pub fn kernel_invoke(handle: HandleID, invocation: Invocation) -> Result<usize, InvocationError> {
    let demanded_rights = invocation.required_rights();

    let table = PRINCIPAL_HANDLE_TABLE.read();
    let obj_arc = table.resolve(handle, demanded_rights)?;
    drop(table);

    obj_arc.invoke(invocation)
}

pub fn kernel_close(handle: HandleID) -> Result<(), InvocationError> {
    let mut table = PRINCIPAL_HANDLE_TABLE.write();
    table.close(handle)
}

pub fn kernel_duplicate(handle: HandleID, requested_rights: AccessRights) -> Result<HandleID, InvocationError> {
    let mut table = PRINCIPAL_HANDLE_TABLE.write();
    let cloned_arc = table.resolve(handle, requested_rights)?;
    Ok(table.insert(cloned_arc, requested_rights))
}

pub fn debug_dump_handles() {
    let table = PRINCIPAL_HANDLE_TABLE.read();
    klogln!("{:#?}", *table);
}

pub fn mount_kernel_dir(name: &str, handle: HandleID, root: HandleID) {
    klog!("Linking {}... ", name);
    // mount '/dev' inside '/'
    kernel_invoke(
        root,
        Invocation::Directory(DirectoryOp::Link { name: name.as_ptr(), name_len: name.len(), handle_id: handle }),
    )
    .expect("Link failed.");
    klogln!("Link success!");
}

