use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::boxed::Box;

use async_trait::async_trait;
use vespertine_abi::op::DirectoryOp;
use vespertine_abi::{
    AccessRights, FileOp, HandleID, Invocation
};

use crate::core::object::invoke::InvocationError;
use crate::core::object::models::process::Process;
use crate::core::object::obj::KernelObject;
use crate::core::sync::KernelOnceCell;
use crate::core::thread::get_current_process;
use crate::klogln;

pub static ROOT_DIRECTORY: KernelOnceCell<Arc<dyn KernelObject>> = KernelOnceCell::new();

pub fn kernel_register_obj(obj: Arc<dyn KernelObject>, init_rights: AccessRights) -> HandleID {
    get_current_process().expect("No active process").proc_handles.write().insert(obj, init_rights)
}

pub async fn kernel_invoke(handle: HandleID, invocation: Invocation) -> Result<usize, InvocationError> {
    let demanded_rights = invocation.required_rights();
    let (obj, rights) = {
        let table = get_current_process().expect("No active processes").proc_handles.read();
        let entry = table.resolve_entry(handle, demanded_rights)?;
        (entry.object.clone(), entry.rights)
    }; // drop the lock 
    obj.invoke(invocation, rights).await
}

pub fn kernel_close(handle: HandleID) -> Result<(), InvocationError> {
    get_current_process().expect("No active process").proc_handles.write().close(handle)
}

pub fn kernel_duplicate(handle: HandleID, requested_rights: AccessRights) -> Result<HandleID, InvocationError> {
    get_current_process().expect("No active process").proc_handles.write().duplicate(handle, requested_rights)
}

pub fn debug_dump_handles() {
    let table = get_current_process().expect("No active process").proc_handles.read();
    klogln!("{:#?}", *table);
}

pub async fn mount_kernel_dir(name: &str, handle: HandleID, root: HandleID) {
    kernel_invoke(root, Invocation::Directory(DirectoryOp::Link { name: name.as_ptr() as usize, name_len: name.len(), handle_id: handle }))
        .await
        .expect("Link failed.");
}

pub async fn kernel_walk(path: &str, handle: HandleID) -> Result<HandleID, InvocationError> {
    let dirs = path.split('/').collect::<Vec<&str>>();
    let start = if dirs[0] == "" { HandleID(0) } else { handle };
    let mut last: HandleID = start;
    for dir in dirs {
        if dir == "" || dir == "." || dir == ".." {
            continue;
        };

        let res = kernel_invoke(last, Invocation::Directory(DirectoryOp::Lookup { name: dir.as_ptr() as usize, name_len: dir.len() })).await;

        if last != start {
            let _ = kernel_close(last);
        }

        last = HandleID(res?);
    }
    Ok(last)
}

pub fn proc_register_obj(proc: &Process, obj: Arc<dyn KernelObject>, rights: AccessRights) -> HandleID {
    proc.proc_handles.write().insert(obj, rights)
}

pub fn proc_cpy_handle(
    src_proc: &Process, src_handle: HandleID, dst_proc: &Process, dst_rights: AccessRights, dst_handle: Option<HandleID>,
) -> Result<HandleID, InvocationError> {
    if let Some(entry) = src_proc.proc_handles.read().get(&src_handle) {
        if let Some(id) = dst_handle {
            dst_proc.proc_handles.write().insert_at(id, entry.object.clone(), dst_rights);
            Ok(id)
        } else {
            Ok(dst_proc.proc_handles.write().insert(entry.object.clone(), dst_rights))
        }
    } else {
        Err(InvocationError::PathNotFound)
    }
}

#[derive(Debug)]
pub struct FileDescription {
    pub inner: Arc<dyn KernelObject>, // The globally shared Ext2File
    pub cursor: AtomicUsize,          // The PER-PROCESS implicit cursor
}

impl FileDescription {
    pub fn new(inner: Arc<dyn KernelObject>) -> Self {
        Self {
            inner,
            cursor: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl KernelObject for FileDescription {
    async fn invoke(&self, mut invocation: Invocation, rights: AccessRights) -> Result<usize, InvocationError> {
        match &mut invocation {
            Invocation::File(FileOp::Read { offset, .. }) => {
                let use_cursor = *offset == usize::MAX; // (uintptr_t)-1 from mlibc
                if use_cursor {
                    *offset = self.cursor.load(Ordering::SeqCst);
                }
                
                // Delegate to Ext2File with the explicit offset
                let bytes = self.inner.invoke(invocation, rights).await?;
                
                if use_cursor {
                    self.cursor.fetch_add(bytes, Ordering::SeqCst);
                }
                Ok(bytes)
            },
            Invocation::File(FileOp::Write { offset, .. }) => {
                let use_cursor = *offset == usize::MAX;
                if use_cursor {
                    *offset = self.cursor.load(Ordering::SeqCst);
                }
                
                let bytes = self.inner.invoke(invocation, rights).await?;
                
                if use_cursor {
                    self.cursor.fetch_add(bytes, Ordering::SeqCst);
                }
                Ok(bytes)
            },
            Invocation::File(FileOp::Seek { offset, whence }) => {
                let current = self.cursor.load(Ordering::SeqCst) as i64;
                let file_size = if *whence == 2 {
                    self.inner
                        .invoke(Invocation::File(FileOp::Stat), rights)
                        .await? as i64
                } else {
                    0
                };
                
                let new_cursor = match *whence {
                    0 => *offset,                                    // SEEK_SET
                    1 => current + *offset,                          // SEEK_CUR
                    2 => file_size + *offset,                        // SEEK_END
                    _ => return Err(InvocationError::InvalidArgument),
                };

                if new_cursor < 0 {
                    return Err(InvocationError::InvalidArgument);
                }

                self.cursor.store(new_cursor as usize, Ordering::SeqCst);
                Ok(new_cursor as usize)
            },
            // Transparently pass through GetVmo, Stat, etc. directly to Ext2File
            _ => self.inner.invoke(invocation, rights).await,
        }
    }
}
