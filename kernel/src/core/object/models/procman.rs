use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr::null;

use async_trait::async_trait;
use vespertine_abi::op::ProcManOp;
use vespertine_abi::{
    AccessRights,
    CapabilityGrant,
    CapabilityID,
    HandleID,
    Invocation,
    ProcessInitPackage,
};

use crate::arch::x86_64::task::syscall::safe_copy_from;
use crate::core::object::handle::HandleTable;
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::mempool::MemPool;
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

pub const DEFAULT_PROCESS_MEMORY_LIMIT: usize = 64 * 1024 * 1024;
pub const DEFAULT_PROCESS_MEMORY_MAXIMUM: usize = 1024 * 1024 * 1024;

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
                cwd_handle,
                cwd_rights,
                source,
                sink,
                capabilities_ptr,
                capabilities_len,
                args_buffer_ptr,
                args_buffer_len,
            }) => {
                calling_rights.err_if_no(AccessRights::CREATE)?;
                let parent_proc = get_current_process().ok_or(InvocationError::OutOfMemory)?;

                let executable = parent_proc.proc_handles.read().resolve(exec_handle, AccessRights::READ | AccessRights::EXECUTE)?;

                let new_proc_root = parent_proc.proc_handles.read().resolve(root_handle, root_rights)?;
                let new_proc_cwd = parent_proc.proc_handles.read().resolve(cwd_handle, cwd_rights)?;

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

                // memory pool handle at 4
                let mem_pool_obj = Arc::new(MemPool::new_expandable(DEFAULT_PROCESS_MEMORY_LIMIT, DEFAULT_PROCESS_MEMORY_MAXIMUM, None));
                new_proc_table.insert_at(HandleID(4), mem_pool_obj, AccessRights::WRITE | AccessRights::CREATE | AccessRights::MUTATE);

                // cwd handle at 5
                new_proc_table.insert_at(HandleID(5), new_proc_cwd, cwd_rights);

                // keep executable file alive for page faults
                new_proc_table.insert(executable, AccessRights::READ);

                // extract handles safely
                let mut child_capabilities = Vec::with_capacity(capabilities_len);

                if capabilities_len > 0 {
                    let mut parent_grants =
                        vec![
                            CapabilityGrant { id: HandleID(0), rights: AccessRights::new(), capability: CapabilityID(0) };
                            capabilities_len
                        ];

                    let success = safe_copy_from(
                        parent_grants.as_mut_ptr() as *mut u8,
                        capabilities_ptr as *const u8,
                        size_of::<CapabilityGrant>() * capabilities_len,
                    );

                    if !success {
                        return Err(InvocationError::InvalidPointer);
                    };

                    for grant in parent_grants {
                        // ensure parent itself has the rights its trying to grant
                        let obj = parent_proc.proc_handles.read().resolve(grant.id, grant.rights)?;
                        // insert into child with attenuated rights
                        let chd = new_proc_table.insert(obj, grant.rights);
                        child_capabilities.push(CapabilityGrant { id: chd, rights: grant.rights, capability: grant.capability });
                    }
                }

                // create the process
                let new_proc = ProcessControlBlock::new(new_proc_table, parent_proc.credentials);

                // load_elf uses the parent's executable_handle since we are in the parent's context
                let load_result = load_elf(exec_handle, &new_proc).await.map_err(|_| InvocationError::InvalidHandle)?;

                // insert self handle at 0 after creating process
                new_proc.proc_handles.write().insert_at(
                    HandleID(1),
                    new_proc.clone(),
                    AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE | AccessRights::CREATE,
                );

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
                let stack_size = 1024 * 1024; // 1 MB
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
                    memory_pool_handle: HandleID(4),
                    cwd_handle: HandleID(5),

                    capabilities_ptr: null(), // inject method sets this, so initialize with null.
                    capabilities_len,

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
                        &child_capabilities,
                        &args_buffer,
                        argc,
                        initpkg,
                        load_result.entry_point,
                        load_result.phdr_addr,
                        load_result.phnum,
                        load_result.base_addr,
                    )?
                };

                // spawn thread, passing the struct pointer as an arg
                let start_ip = load_result.interpreter_entry.unwrap_or(load_result.entry_point);
                spawn_user_thread(start_ip, safe_stack_top, pkg_vaddr, ThreadPriority::MEDIUM, new_proc.clone());

                let new_handle_id =
                    parent_proc.proc_handles.write().insert(new_proc, AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);

                Ok(new_handle_id.0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
