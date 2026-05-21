use alloc::sync::Arc;

use crate::arch::get_core_data;
use crate::kernel::object::handle::{
    AccessRights,
    HandleID,
};
use crate::kernel::object::invoke::{
    Invocation,
    InvocationError,
};
use crate::kernel::object::models::directory::Directory;
use crate::kernel::object::obj::{
    KernelHandleTable,
    KernelObject,
};
use crate::kernel::object::op::DirectoryOp;
use crate::kernel::sync::{KernelOnceCell, RwLock};
use crate::{
    klog, klogln
};

pub static ROOT_DIRECTORY: KernelOnceCell<Arc<Directory>> = KernelOnceCell::new();

pub fn kernel_register_obj(obj: Arc<dyn KernelObject>, init_rights: AccessRights) -> HandleID {
    let mut table = PRINCIPAL_HANDLE_TABLE.write();
    table.insert(obj, init_rights)
}

pub fn kernel_invoke(handle: HandleID, invocation: Invocation) -> Result<usize, InvocationError> {
    let demanded_rights = invocation.required_rights();

    let current_thread = get_core_data().scheduler.get_current_thread();
    let process = unsafe { &(*current_thread).process };

    let table = process.proc_handles.read();
    let entry = table.get(&handle).ok_or(InvocationError::InvalidHandle)?;

    if !entry.rights.contains(demanded_rights) {
        return Err(InvocationError::AccessDenied);
    }

    let obj_arc = entry.object.clone();
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

