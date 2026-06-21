use alloc::boxed::Box;

use async_trait::async_trait;
use vespertine_abi::{
    AccessRights,
    Invocation,
    ThreadOp,
};

use crate::arch::get_core_data;
use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::core::cpu::get_core_data_for;
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::obj::KernelObject;
use crate::core::thread::schedule::{
    GRAVEYARD,
    ScheduleReason,
};
use crate::core::thread::{
    ThreadControlBlock,
    ThreadState,
};

#[derive(Debug)]
pub struct Thread {
    pub tcb: *mut ThreadControlBlock,
}

unsafe impl Sync for Thread {}
unsafe impl Send for Thread {}

#[async_trait]
impl KernelObject for Thread {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Thread(ThreadOp::Kill) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                unsafe {
                    (*self.tcb).set_state(ThreadState::Terminated);
                    GRAVEYARD.lock().push(self.tcb);
                    let assigned_core = (*self.tcb).assigned_core();
                    let this_core = get_core_data().logical_id;
                    if assigned_core != this_core {
                        let tgt = get_core_data_for(assigned_core);
                        get_core_data().apic_mode.send_ipi(tgt.lapic_id as u32, 40);
                    } else {
                        get_core_data().scheduler.schedule(ScheduleReason::Terminated);
                    }
                }
                Ok(0)
            }
            Invocation::Thread(ThreadOp::Join) => Err(InvocationError::UnsupportedOperation),
            Invocation::Thread(ThreadOp::GetID) => {
                calling_rights.err_if_no(AccessRights::READ)?;
                let id = unsafe { (*self.tcb).thread_id };
                Ok(id)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn type_name(&self) -> &'static str { "Thread" }
}
