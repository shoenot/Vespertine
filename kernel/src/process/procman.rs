use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr::{
    copy_nonoverlapping,
    null,
};
use core::sync::atomic::Ordering;

use async_trait::async_trait;
use hal::usercopy::safe_copy_from;
use vespertine_abi::op::ProcManOp;
use vespertine_abi::protocol::{
    PacketFlags,
    PacketHeader,
    PacketType,
    VESPER_MAGIC,
};
use vespertine_abi::{
    AccessRights,
    CapabilityGrant,
    CapabilityID,
    FileOp,
    HandleID,
    Invocation,
    PROC_NAME_LEN_MAX,
    ProcInfo,
    ProcessInitPackage,
    SpawnCredentials,
};

use crate::executor::Executor;
use crate::memory::PAGE_SIZE_4K;
use crate::object::handle::HandleTable;
use crate::object::help::RightsWrapper;
use crate::object::invoke::InvocationError;
use crate::object::mempool::MemPool;
use crate::process::{
    ProcessControlBlock,
    find_process,
    process_snapshot,
    register_process,
};
use crate::object::obj::KernelObject;
use crate::program::env::ProcessEnvironment;
use crate::program::{LoaderError, load_elf};
use crate::security::credentials::Credentials;
use crate::sched::dispatch::{
    create_user_tcb, create_user_thread_suspended, spawn_user_thread,
};
use crate::process::current_process;
use crate::sched::priority::ThreadPriority;
use crate::memory::vmm::{
    VmOptions, VmaBacking,
};
use crate::memory::vmo::AnonymousVmo;

#[derive(Debug)]
pub struct ProcessManager {}

#[async_trait]
impl KernelObject for ProcessManager {
    fn type_name(&self) -> &'static str { "Process Manager" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::ProcessManager(ProcManOp::Spawn {
                name_ptr,
                name_len,
                exec_handle,
                root_handle,
                root_rights,
                cwd_handle,
                cwd_rights,
                source,
                sink,
                credentials,
                capabilities_ptr,
                capabilities_len,
                args_buffer_ptr,
                args_buffer_len,
                start_suspended,
            }) => {
                calling_rights.err_if_no(AccessRights::CREATE)?;
                let parent_proc = current_process().ok_or(InvocationError::OutOfMemory)?;

                let executable = parent_proc.handles.read().resolve(exec_handle, AccessRights::READ | AccessRights::EXECUTE)?;

                let new_proc_root = parent_proc.handles.read().resolve(root_handle, root_rights)?;
                let new_proc_cwd = parent_proc.handles.read().resolve(cwd_handle, cwd_rights)?;

                let grant_bytes = capabilities_len
                    .checked_mul(size_of::<CapabilityGrant>())
                    .ok_or(InvocationError::InvalidArgument)?;

                if grant_bytes.checked_add(args_buffer_len)
                    .is_none_or(|bytes| bytes > PAGE_SIZE_4K) {
                        return Err(InvocationError::OutOfMemory);
                }

                let mut new_proc_table = HandleTable::new(); // create a blank table

                // root handle at 1
                new_proc_table.insert_at(HandleID(0), new_proc_root, root_rights);

                // source handle at 2
                if let Ok(source_obj) = parent_proc.handles.read().resolve(source, AccessRights::READ) {
                    new_proc_table.insert_at(HandleID(2), source_obj, AccessRights::READ);
                }

                // sink handle at 3
                if let Ok(sink_obj) = parent_proc.handles.read().resolve(sink, AccessRights::WRITE) {
                    new_proc_table.insert_at(HandleID(3), sink_obj, AccessRights::WRITE);
                }

                // memory pool at handle 4 created by ProcessControlBlock::new_unregistered() now

                // cwd handle at 5
                new_proc_table.insert_at(HandleID(5), new_proc_cwd, cwd_rights);

                // extract handles safely
                let mut child_capabilities = Vec::new();
                child_capabilities.try_reserve_exact(capabilities_len).map_err(|_| InvocationError::OutOfMemory)?;

                if capabilities_len > 0 {
                    let mut parent_grants = Vec::new();
                    parent_grants.try_reserve_exact(capabilities_len)
                        .map_err(|_| InvocationError::OutOfMemory)?;
                    parent_grants.resize(
                        capabilities_len,
                        CapabilityGrant {
                            id: HandleID(0),
                            rights: AccessRights::new(),
                            capability: CapabilityID(0),
                        },
                    );

                    if !safe_copy_from(
                        parent_grants.as_mut_ptr().cast::<u8>(),
                        capabilities_ptr as *const u8,
                        grant_bytes,
                    ) {
                        return Err(InvocationError::InvalidPointer);
                    }

                    for grant in parent_grants {
                        // ensure parent itself has the rights its trying to grant
                        let obj = parent_proc.handles.read().resolve(grant.id, grant.rights)?;
                        // insert into child with attenuated rights
                        let chd = new_proc_table.insert(obj, grant.rights);
                        child_capabilities.push(CapabilityGrant { id: chd, rights: grant.rights, capability: grant.capability });
                    }
                }

                // create the process
                let child_credentials = match credentials {
                    SpawnCredentials::Inherit => parent_proc.credentials,
                    SpawnCredentials::User { user } => {
                        if !parent_proc.credentials.is_system() {
                            return Err(InvocationError::AccessDenied);
                        }

                        Credentials::new(user)
                    }
                };

                if name_len == 0 || name_len > PROC_NAME_LEN_MAX {
                    return Err(InvocationError::InvalidArgument);
                }
                let mut name_bytes = Vec::new();
                if name_bytes.try_reserve_exact(name_len).is_err() {
                    return Err(InvocationError::OutOfMemory);
                }
                name_bytes.resize(name_len, 0);

                if !safe_copy_from(name_bytes.as_mut_ptr(), name_ptr as *const u8, name_len) {
                    return Err(InvocationError::InvalidPointer);
                }

                let name_str = String::from_utf8(name_bytes).map_err(|_| InvocationError::InvalidArgument)?;

                // create the process
                let new_proc = ProcessControlBlock::new_unregistered(new_proc_table, name_str, child_credentials)?;

                // load_elf uses the parent's executable_handle since we are in the parent's context
                let load_result = load_elf(executable, &new_proc).await
                    .map_err(|error| match error {
                        LoaderError::OutOfMemory => InvocationError::OutOfMemory,
                        _ => InvocationError::InvalidHandle,
                    })?;

                let mut args_buffer = Vec::new();
                args_buffer.try_reserve_exact(args_buffer_len).map_err(|_| InvocationError::OutOfMemory)?;
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
                let stack_vmo = AnonymousVmo::new(stack_size);

                let stack_addr = new_proc
                    .vmm
                    .write()
                    .as_mut().ok_or(InvocationError::InvalidHandle)?
                    .reserve(stack_size, VmOptions::user_rw(), VmaBacking::Vmo(stack_vmo.clone()))
                    .map_err(|_| InvocationError::OutOfMemory)?;

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
                let thread = create_user_tcb(
                    start_ip,
                    safe_stack_top,
                    pkg_vaddr,
                    ThreadPriority::MEDIUM,
                    new_proc.clone(),
                )?;

                new_proc.handles.write().insert_at(HandleID(1), new_proc.clone(), 
                    AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE | AccessRights::CREATE);

                unsafe { new_proc.install_initial_thread(thread) };
                register_process(&new_proc);

                let handle = parent_proc.handles.write().insert(new_proc.clone(), AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);

                if !start_suspended { new_proc.resume_initial_thread(); }

                Ok(handle.0)
            }
            Invocation::ProcessManager(ProcManOp::List { offset, sink }) => {
                calling_rights.err_if_no(AccessRights::LIST)?;

                let processes = process_snapshot();
                let entries: Vec<ProcInfo> = processes.iter().skip(offset).map(|process| process.snapshot_info()).collect();

                let proc = current_process().ok_or(InvocationError::InvalidHandle)?;
                let sink_obj = proc.handles.read().resolve(sink, AccessRights::WRITE)?;

                Executor::new().spawn(async move {
                    let mut iter = entries.iter().peekable();

                    while let Some(info) = iter.next() {
                        let header = PacketHeader {
                            magic: VESPER_MAGIC,
                            version: 1,
                            packet_flags: PacketFlags::IS_STREAM,
                            packet_type: PacketType::ProcessInfo as u32,
                            payload_len: size_of::<ProcInfo>() as u32,
                            reserved: 0,
                        };

                        let mut buffer = [0u8; size_of::<PacketHeader>() + size_of::<ProcInfo>()];
                        let header_size = size_of::<PacketHeader>();
                        let entry_size = size_of::<ProcInfo>();

                        unsafe {
                            let header_ptr = &header as *const _ as *const u8;
                            let info_ptr = info as *const ProcInfo as *const u8;

                            copy_nonoverlapping(header_ptr, buffer.as_mut_ptr(), header_size);
                            copy_nonoverlapping(info_ptr, buffer.as_mut_ptr().add(header_size), entry_size);
                        }

                        if write_all_to_object(&sink_obj, AccessRights::WRITE, &buffer).await.is_err() {
                            return;
                        }
                    }

                    let header = PacketHeader {
                        magic: VESPER_MAGIC,
                        version: 1,
                        packet_flags: PacketFlags::IS_STREAM,
                        packet_type: PacketType::ProcessInfo as u32,
                        payload_len: 0,
                        reserved: 0,
                    };

                    let mut buffer = [0u8; size_of::<PacketHeader>()];

                    unsafe {
                        let header_ptr = &header as *const _ as *const u8;
                        copy_nonoverlapping(header_ptr, buffer.as_mut_ptr(), size_of::<PacketHeader>());
                    }

                    let _ = write_all_to_object(&sink_obj, AccessRights::WRITE, &buffer).await;
                });

                Ok(processes.len())
            }
            Invocation::ProcessManager(ProcManOp::Open { pid, rights }) => {
                calling_rights.err_if_no(AccessRights::READ)?;

                let process = find_process(pid).ok_or(InvocationError::InvalidHandle)?;
                let granted = rights & (AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);

                if granted == AccessRights::new() {
                    return Err(InvocationError::AccessDenied);
                }

                let caller = current_process().ok_or(InvocationError::InvalidHandle)?;
                let handle = caller.handles.write().insert(process, granted);
                Ok(handle.0)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

async fn write_all_to_object(object: &Arc<dyn KernelObject>, rights: AccessRights, bytes: &[u8]) -> Result<(), InvocationError> {
    let mut written = 0;

    while written < bytes.len() {
        let op = FileOp::Write { offset: 0, buffer_ptr: unsafe { bytes.as_ptr().add(written) as usize }, len: bytes.len() - written };

        let count = object.invoke(Invocation::File(op), rights).await?;
        if count == 0 {
            return Err(InvocationError::UnsupportedOperation);
        }

        written += count;
    }
    Ok(())
}
