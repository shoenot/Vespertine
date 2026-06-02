use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr::null;

use async_trait::async_trait;
use vespertine_abi::op::ProcManOp;
use vespertine_abi::{
    AccessRights,
    HandleGrant,
    HandleID,
    Invocation,
    ProcessInitPackage,
};

use crate::arch::x86_64::task::syscall::safe_copy_from;
use crate::core::object::handle::HandleTable;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::process::ProcessControlBlock;
use crate::core::object::obj::KernelObject;
use crate::core::program::env::ProcessEnvironment;
use crate::core::program::load_elf;
use crate::core::thread::dispatch::spawn_user_thread;
use crate::core::thread::get_current_process;
use crate::core::thread::priority::ThreadPriority;
use crate::memory::vmm::{
    VM_FLAG_USER,
    VM_FLAG_WRITE,
};
use crate::memory::vmo::Vmo;

#[derive(Debug)]
pub struct ProcessManager {}

#[async_trait]
impl KernelObject for ProcessManager {
    fn type_name(&self) -> &'static str { "Process Manager" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::ProcessManager(ProcManOp::Spawn {
                exec_handle,
                root_handle,
                root_rights,
                source,
                sink,
                extra_handles_ptr,
                extra_handles_len,
                args_buffer_ptr,
                args_buffer_len,
            }) => {
                if !calling_rights.contains(AccessRights::CREATE) {
                    return Err(InvocationError::AccessDenied);
                }

                let parent_proc = get_current_process().ok_or(InvocationError::OutOfMemory)?;
                let new_proc_root = parent_proc.proc_handles.read().resolve(root_handle, root_rights)?;

                let mut new_proc_table = HandleTable::new(); // create a blank table

                // root handle at 1
                new_proc_table.insert_at(HandleID(0), new_proc_root, root_rights);

                // source handle at 2
                if let Ok(source_obj) = parent_proc.proc_handles.read().resolve(source, AccessRights::READ) {
                    new_proc_table.insert_at(HandleID(2), source_obj, AccessRights::READ);
                }

                // sink handle at 3
                if let Ok(sink_obj) = parent_proc.proc_handles.read().resolve(sink, AccessRights::WRITE) {
                    new_proc_table.insert_at(HandleID(3), sink_obj, AccessRights::WRITE);
                }

                // extract handles safely
                let mut child_extra_handles = Vec::with_capacity(extra_handles_len);

                if extra_handles_len > 0 {
                    let mut parent_grants =
                        vec![vespertine_abi::HandleGrant { id: HandleID(0), rights: AccessRights::new(), tag: 0 }; extra_handles_len];
                    let success = safe_copy_from(
                        parent_grants.as_mut_ptr() as *mut u8,
                        extra_handles_ptr as *const u8,
                        core::mem::size_of::<vespertine_abi::HandleGrant>() * extra_handles_len,
                    );

                    if !success {
                        return Err(InvocationError::InvalidPointer);
                    };

                    for grant in parent_grants {
                        // ensure parent itself has the rights its trying to grant
                        if let Ok(obj) = parent_proc.proc_handles.read().resolve(grant.id, grant.rights) {
                            // insert into child with attenuated rights
                            let chd = new_proc_table.insert(obj, grant.rights);
                            child_extra_handles.push(HandleGrant { id: chd, rights: grant.rights, tag: grant.tag });
                        } else {
                            return Err(InvocationError::InvalidHandle);
                        }
                    }
                }

                // create the process
                let new_proc = ProcessControlBlock::new(new_proc_table);

                // insert self handle at 0 after creating process
                new_proc.proc_handles.write().insert_at(
                    HandleID(1),
                    new_proc.clone(),
                    AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE,
                );

                // load_elf uses the parent's executable_handle since we are in the parent's context
                let (entry_point, phdr_addr, phnum) =
                    { load_elf(exec_handle, &new_proc).await.map_err(|_| InvocationError::InvalidHandle)? };

                let mut args_buffer = Vec::with_capacity(args_buffer_len);
                let mut argc = 0;

                if args_buffer_len > 0 {
                    args_buffer.resize(args_buffer_len, 0);
                    let success = safe_copy_from(args_buffer.as_mut_ptr() as *mut u8, args_buffer_ptr as *const u8, args_buffer_len);
                    if !success {
                        return Err(InvocationError::InvalidPointer);
                    }

                    // count null terminators to determine argc
                    for &b in &args_buffer {
                        if b == 0 {
                            argc += 1;
                        }
                    }
                }

                // stack building
                let stack_size = 8192 * 2; // cba to calculate 16 kbs, fix later
                let stack_vmo = Vmo::new(stack_size);

                let stack_addr = new_proc
                    .vmm
                    .write()
                    .mmap_vmo(stack_size, VM_FLAG_USER | VM_FLAG_WRITE, stack_vmo.clone())
                    .ok_or(InvocationError::OutOfMemory)?;

                let initpkg = ProcessInitPackage {
                    root_handle: HandleID(0),
                    self_handle: HandleID(1),
                    source_handle: HandleID(2),
                    sink_handle: HandleID(3),
                    extra_handles_ptr: null(), // inject method sets this, so initialize with null.
                    extra_handles_len,
                    argc: 0,
                    argv: null(), // same as above
                    envp: null(),  
                };

                // inject the payload
                let (pkg_vaddr, safe_stack_top) = {
                    ProcessEnvironment::inject(
                        &stack_vmo,
                        stack_addr,
                        stack_size,
                        &child_extra_handles,
                        &args_buffer,
                        argc,
                        initpkg,
                        entry_point,
                        phdr_addr,
                        phnum,
                    )?
                };

                // spawn thread, passing the struct pointer as an arg
                spawn_user_thread(entry_point, safe_stack_top, pkg_vaddr, ThreadPriority::MEDIUM, new_proc.clone());

                let new_handle_id =
                    parent_proc.proc_handles.write().insert(new_proc, AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);

                Ok(new_handle_id.0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
