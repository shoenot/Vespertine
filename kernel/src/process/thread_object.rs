use alloc::boxed::Box;

use alloc::sync::Arc;
use async_trait::async_trait;
use hal::ipi::send_reschedule_ipi;
use vespertine_abi::{
    AccessRights,
    Invocation,
    ThreadOp,
};

use crate::cpu::current_core_id;
use crate::klogln;
use crate::object::help::RightsWrapper;
use crate::object::invoke::InvocationError;
use crate::object::obj::KernelObject;
use crate::sync::TicketLock;
use crate::sched::{Thread, ThreadState};

#[derive(Debug)]
struct LiveTcb(*mut Thread);

unsafe impl Send for LiveTcb {}

#[derive(Debug)]
pub struct ThreadObject {
    tid: usize,
    live: TicketLock<Option<LiveTcb>>,
}

impl ThreadObject {
    /// the caller must supply a valid tcb that has never been enqueued and attach at most one obj
    /// to it.
    pub unsafe fn attach(tcb: *mut Thread) -> Arc<Self> {
        let object = Arc::new(Self {
            tid: unsafe { (*tcb).thread_id },
            live: TicketLock::new(Some(LiveTcb(tcb))),
        });

        unsafe { (*tcb).handle = Some(Arc::clone(&object)) };
        object
    }

    pub fn detach(&self) {
        *self.live.lock() = None;
    }

    fn request_kill(&self) {
        let target_core = {
            let live = self.live.lock();
            let Some(tcb) = live.as_ref() else {
                return;
            };

            unsafe {
                if (*tcb.0).state() == ThreadState::Terminated {
                    return;
                }

                (*tcb.0).request_cancel();
                (*tcb.0).assigned_core()
            }
        };

        if target_core != current_core_id() {
            send_reschedule_ipi(target_core);
        }
    }
}

#[async_trait]
impl KernelObject for ThreadObject {
    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Thread(ThreadOp::Kill) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.request_kill();
                Ok(0)
            }
            Invocation::Thread(ThreadOp::Join) => Err(InvocationError::UnsupportedOperation),
            Invocation::Thread(ThreadOp::GetID) => {
                calling_rights.err_if_no(AccessRights::READ)?;
                Ok(self.tid)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn type_name(&self) -> &'static str { "Thread" }
}
