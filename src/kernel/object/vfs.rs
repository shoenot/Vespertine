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
use crate::kernel::sync::RwLock;
use crate::{
    klogln,
};

pub static PRINCIPAL_HANDLE_TABLE: RwLock<KernelHandleTable> = RwLock::new(KernelHandleTable::new());


pub fn kernel_register_obj(obj: Arc<dyn KernelObject>, init_rights: AccessRights) -> HandleID {
    let mut table = PRINCIPAL_HANDLE_TABLE.write();
    table.insert(obj, init_rights)
}

pub fn sys_invoke(handle: HandleID, invocation: Invocation) -> Result<usize, InvocationError> {
    let demanded_rights = invocation.required_rights();

    let table = PRINCIPAL_HANDLE_TABLE.read();
    let obj_arc = table.resolve(handle, demanded_rights)?;
    drop(table);

    obj_arc.invoke(invocation)
}

pub fn sys_close(handle: HandleID) -> Result<(), InvocationError> {
    let mut table = PRINCIPAL_HANDLE_TABLE.write();
    table.close(handle)
}

pub fn sys_duplicate(handle: HandleID, requested_rights: AccessRights) -> Result<HandleID, InvocationError> {
    let mut table = PRINCIPAL_HANDLE_TABLE.write();
    let cloned_arc = table.resolve(handle, requested_rights)?;
    Ok(table.insert(cloned_arc, requested_rights))
}

pub fn debug_dump_handles() {
    let table = PRINCIPAL_HANDLE_TABLE.read();
    klogln!("{:#?}", *table);
}

