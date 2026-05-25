use crate::{KERNEL_PROCESS, core::{object::{handle::HandleTable, invoke::InvocationError, models::process::ProcessControlBlock, obj::KernelObject, vfs::kernel_walk}, program::load_elf, thread::{dispatch::spawn_user_thread, get_current_process, priority::ThreadPriority}}, memory::vmm::{VM_FLAG_USER, VM_FLAG_WRITE}};
use vespertine_abi::Invocation;

use vespertine_abi::op::ProcManOp;
use vespertine_abi::{AccessRights, HandleID};

#[derive(Debug)]
pub struct ProcessManager {}

impl KernelObject for ProcessManager {
    fn type_name(&self) -> &'static str {
        "Process Manager"
    }

    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::ProcessManager(ProcManOp::Spawn { exec_handle, root_handle, root_rights, source, sink }) => {
                if !calling_rights.contains(AccessRights::CREATE) {
                    return Err(InvocationError::AccessDenied);
                }

                let parent_proc = get_current_process().ok_or(InvocationError::OutOfMemory)?;
                let new_proc_root = parent_proc.proc_handles.read().resolve(root_handle, root_rights)?;

                let mut new_proc_table = HandleTable::new();   // create a blank table

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

                // create the process
                let new_proc = ProcessControlBlock::new(new_proc_table);

                // insert self handle at 0 after creating process
                new_proc.proc_handles.write().insert_at(
                    HandleID(1), new_proc.clone(), AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE
                );

                // load_elf uses the parent's executable_handle since we are in the parent's context
                let entry_point = load_elf(exec_handle, &new_proc).map_err(|_| InvocationError::InvalidHandle)?; 

                let stack_size = 8192 * 2; // cba to calculate 16 kbs, fix later
                let stack_addr = new_proc.vmm.write()
                    .mmap(stack_size, VM_FLAG_USER | VM_FLAG_WRITE).ok_or(InvocationError::OutOfMemory)?;
                let user_stack_top = stack_addr + stack_size;

                // spawn init thread
                spawn_user_thread(entry_point, user_stack_top, 0, ThreadPriority::MEDIUM, new_proc.clone());

                let new_handle_id = parent_proc.proc_handles.write()
                    .insert(new_proc, AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);

                Ok(new_handle_id.0)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
