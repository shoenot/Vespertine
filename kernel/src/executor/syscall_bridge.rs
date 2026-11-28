use alloc::sync::Arc;
use alloc::task::Wake;
use core::pin::pin;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};
use core::task::{
    Context,
    Poll,
};

use hal::interrupts::{
    disable_interrupts,
    enable_interrupts,
    interrupts_enabled,
};
use vespertine_abi::{
    HandleID,
    Invocation,
};

use crate::cpu::current_core_mut;
use crate::object::invoke::{InvocationError, InvokeContext};
use crate::object::vfs::kernel_invoke;
use crate::sync::TicketLock;
use crate::sched::block::{ThreadWakeRegistration, WakeTarget};
use crate::sched::dispatch::resume_registered_if_awoken;
use crate::sched::scheduler::ScheduleReason;

struct ThreadWaker {
    target: Arc<WakeTarget>,
    registration: TicketLock<Arc<ThreadWakeRegistration>>,
    awoken: AtomicBool,
}

impl ThreadWaker {
    fn new(target: Arc<WakeTarget>) -> Arc<Self> {
        let reg = ThreadWakeRegistration::new(Arc::clone(&target));
        Arc::new(Self { target, registration: TicketLock::new(reg), awoken: AtomicBool::new(false) })
    }

    fn arm(&self) -> Arc<ThreadWakeRegistration> {
        let reg = ThreadWakeRegistration::new(Arc::clone(&self.target));
        *self.registration.lock() = reg.clone();
        reg
    }
}

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) { self.wake_by_ref(); }

    fn wake_by_ref(self: &Arc<Self>) {
        self.awoken.store(true, Ordering::Release);
        let registration = Arc::clone(&*self.registration.lock());
        registration.wake();
    }
}

pub fn handle_sys_invoke(handle: HandleID, invocation: Invocation) -> Result<usize, InvocationError> {
    let tcb = current_core_mut().scheduler.get_current_thread();
    let thread = unsafe { &*tcb };
    let ctx = InvokeContext::for_thread(thread.termination_token());
    block_on(kernel_invoke(handle, invocation, ctx))
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let tcb = current_core_mut().scheduler.get_current_thread();
    let waker_inner = ThreadWaker::new(unsafe { Arc::clone(&(*tcb).wake_target) });
    let waker = waker_inner.clone().into();
    let mut context = Context::from_waker(&waker);

    loop {
        waker_inner.awoken.store(false, Ordering::Release);
        let registration = waker_inner.arm();

        match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => {
                registration.cancel();
                return result;
            }
            Poll::Pending => {
                let int_state = interrupts_enabled();
                disable_interrupts();

                let thread = unsafe { &*tcb };
                thread.park_registration(Arc::clone(&registration));

                if !resume_registered_if_awoken(thread, &registration, &waker_inner.awoken) {
                    current_core_mut().scheduler.schedule(ScheduleReason::Blocked);
                }

                registration.cancel();
                if int_state { enable_interrupts() };
            }
        }
    }
}
