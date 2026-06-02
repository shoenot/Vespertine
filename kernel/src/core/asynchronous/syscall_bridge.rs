use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;
use core::pin::Pin;
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
use crate::core::thread::dispatch::wake_thread;
use crate::core::thread::{
    ThreadControlBlock,
    ThreadState,
};

struct ThreadWaker {
    thread: *mut ThreadControlBlock,
}

unsafe impl Send for ThreadWaker {}
unsafe impl Sync for ThreadWaker {}

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        unsafe {
            if (*self.thread).state == ThreadState::Blocked {
                wake_thread(self.thread);
            }
        }
    }
}

pub fn handle_sys_invoke(handle: HandleID, invocation: Invocation) -> Result<usize, InvocationError> {
    let tcb = get_core_data().scheduler.get_current_thread();
    let mut future = Box::pin(kernel_invoke(handle, invocation));

    let waker = Arc::new(ThreadWaker { thread: tcb }).into();
    let mut context = Context::from_waker(&waker);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => {
                let int_state = interrupts_enabled();
                disable_interrupts();
                
                unsafe {
                    if (*tcb).state == ThreadState::Running {
                        (*tcb).state = ThreadState::Blocked;
                        let sched = &mut get_core_data().scheduler;
                        sched.schedule();
                    } else {
                        (*tcb).state = ThreadState::Ready;
                        let sched = &mut get_core_data().scheduler;
                        sched.push(tcb);
                        sched.schedule();
                    }
                }
                
                if int_state {
                    enable_interrupts();
                }
            }
        }
    }
}

pub fn block_on<F: Future>(mut future: Pin<Box<F>>) -> F::Output {
    let tcb = get_core_data().scheduler.get_current_thread();
    let waker = Arc::new(ThreadWaker { thread: tcb }).into();
    let mut context = Context::from_waker(&waker);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => {
                let int_state = interrupts_enabled();
                disable_interrupts();
                
                unsafe {
                    if (*tcb).state == ThreadState::Running {
                        (*tcb).state = ThreadState::Blocked;
                        let sched = &mut get_core_data().scheduler;
                        sched.schedule();
                    } else {
                        (*tcb).state = ThreadState::Ready;
                        let sched = &mut get_core_data().scheduler;
                        sched.push(tcb);
                        sched.schedule();
                    }
                }
                
                if int_state {
                    enable_interrupts();
                };
            }
        }
    }
}
