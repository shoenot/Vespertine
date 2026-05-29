use vespertine_abi::{AccessRights, Invocation, ThreadOp};

use crate::{arch::{get_core_data, x86_64::apic::lapic::ApicDriver}, core::{cpu::get_core_data_for, object::{invoke::InvocationError, obj::KernelObject}, thread::{ThreadControlBlock, ThreadState, dispatch::wake_thread, schedule::GRAVEYARD}}, terminate_thread};

#[derive(Debug)]
pub struct Thread {
    pub tcb: *mut ThreadControlBlock,
}

unsafe impl Sync for Thread {}
unsafe impl Send for Thread {}

impl KernelObject for Thread {
    fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Thread(ThreadOp::Kill) => {
                if !calling_rights.contains(AccessRights::WRITE) { return Err(InvocationError::AccessDenied) };
                unsafe {
                    (*self.tcb).state = ThreadState::Terminated;
                    GRAVEYARD.lock().push(self.tcb);
                    let home_core = (*self.tcb).home_core;
                    let this_core = get_core_data().logical_id;
                    if home_core != this_core {
                        let tgt = get_core_data_for(home_core);
                        get_core_data().apic_mode.send_ipi(tgt.lapic_id as u32, 64);
                    } else {
                        get_core_data().scheduler.schedule();
                    }
                }
                Ok(0)
            },
            Invocation::Thread(ThreadOp::Join) => {
                Err(InvocationError::UnsupportedOperation)
            },
            Invocation::Thread(ThreadOp::GetID) => {
                if !calling_rights.contains(AccessRights::READ) { return Err(InvocationError::AccessDenied) };
                let id = unsafe { (*self.tcb).thread_id };
                Ok(id)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn type_name(&self) -> &'static str {
        "Thread"
    }
}
