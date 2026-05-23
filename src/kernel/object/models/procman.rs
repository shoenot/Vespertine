use crate::{kernel::{object::{handle::AccessRights, invoke::{Invocation, InvocationError}, models::process::ProcessControlBlock, obj::KernelObject, op::ProcManOp}, program::load_elf, thread::{dispatch::spawn_user_thread, get_current_process, priority::ThreadPriority}}, memory::vmm::{VM_FLAG_USER, VM_FLAG_WRITE}};


#[derive(Debug)]
pub struct ProcessManager {}

impl KernelObject for ProcessManager {
    fn type_name(&self) -> &'static str {
        "Process Manager"
    }

    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::ProcessManager(ProcManOp::Spawn { exec_handle, root_handle, root_rights }) => {
                if !calling_rights.contains(AccessRights::CREATE) {
                    return Err(InvocationError::AccessDenied);
                }

                let parent_proc = get_current_process().ok_or(InvocationError::OutOfMemory)?;

                let child_root = parent_proc.proc_handles.read().resolve(root_handle, root_rights)?;

                let child_proc = ProcessControlBlock::new(child_root, root_rights);

                // load_elf uses the parent's executable_handle since we are in the parent's context
                let entry_point = load_elf(exec_handle, &child_proc).map_err(|_| InvocationError::InvalidHandle)?; 

                let stack_size = 8192;
                let stack_addr = child_proc.vmm.write()
                    .mmap(stack_size, VM_FLAG_USER | VM_FLAG_WRITE).ok_or(InvocationError::OutOfMemory)?;
                let user_stack_top = stack_addr + stack_size;

                // spawn init thread
                spawn_user_thread(entry_point, user_stack_top, 0, ThreadPriority::MEDIUM, child_proc.clone());

                let child_handle_id = parent_proc.proc_handles.write()
                    .insert(child_proc, AccessRights::READ | AccessRights::WRITE | AccessRights::MUTATE);

                Ok(child_handle_id.0)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
