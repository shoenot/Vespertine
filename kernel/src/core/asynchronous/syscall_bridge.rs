use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;
use core::pin::Pin;
use core::sync::atomic::{
    AtomicBool,
    Ordering,
};
use core::task::{
    Context,
    Poll,
};

use vespertine_abi::{
    HandleID,
    Invocation,
};

use crate::arch::{
    disable_interrupts,
    enable_interrupts,
    get_core_data,
    interrupts_enabled,
};
use crate::core::object::invoke::InvocationError;
use crate::core::object::vfs::kernel_invoke;
use crate::core::thread::block::ThreadWakeRegistration;
use crate::core::thread::dispatch::{
    cancel_block_if_awoken,
    wake_thread,
};
use crate::core::thread::schedule::ScheduleReason;
use crate::core::thread::{
    ThreadControlBlock,
    ThreadState,
};

struct ThreadWaker {
    registration: Arc<ThreadWakeRegistration>,
    awoken: AtomicBool,
}

unsafe impl Send for ThreadWaker {}
unsafe impl Sync for ThreadWaker {}

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) { 
        self.awoken.store(true, Ordering::Release);
        self.registration.wake(); 
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.awoken.store(true, Ordering::Release);
        self.registration.wake(); 
    }
}

pub fn handle_sys_invoke(handle: HandleID, invocation: Invocation) -> Result<usize, InvocationError> {
    let tcb = get_core_data().scheduler.get_current_thread();
    let mut future = Box::pin(kernel_invoke(handle, invocation));

    let waker_inner = Arc::new(ThreadWaker { thread: tcb, awoken: core::sync::atomic::AtomicBool::new(false) });
    let waker = waker_inner.clone().into();
    let mut context = Context::from_waker(&waker);

    loop {
        waker_inner.awoken.store(false, Ordering::Release);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => {
                let int_state = interrupts_enabled();
                disable_interrupts();
                let thread = unsafe { &*tcb };

                thread.transition(ThreadState::Running, ThreadState::Blocked).expect("current thread was not running");

                if cancel_block_if_awoken(thread, &waker_inner.awoken) {
                    if int_state {
                        enable_interrupts();
                    }
                    continue;
                }

                let sched = &mut get_core_data().scheduler;
                sched.schedule(ScheduleReason::Blocked);
                if int_state {
                    enable_interrupts();
                }
            }
        }
    }
}

pub fn block_on<F: Future>(mut future: Pin<Box<F>>) -> F::Output {
    let tcb = get_core_data().scheduler.get_current_thread();
    let waker_inner = Arc::new(ThreadWaker { thread: tcb, awoken: core::sync::atomic::AtomicBool::new(false) });
    let waker = waker_inner.clone().into();
    let mut context = Context::from_waker(&waker);

    loop {
        waker_inner.awoken.store(false, Ordering::Release);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => {
                let int_state = interrupts_enabled();
                disable_interrupts();

                let thread = unsafe { &*tcb };
                thread.transition(ThreadState::Running, ThreadState::Blocked).expect("current thread was not running");

                if cancel_block_if_awoken(thread, &waker_inner.awoken) {
                    if int_state {
                        enable_interrupts();
                    }
                    continue;
                }

                let sched = &mut get_core_data().scheduler;
                sched.schedule(ScheduleReason::Blocked);

                if int_state {
                    enable_interrupts();
                };
            }
        }
    }
}
