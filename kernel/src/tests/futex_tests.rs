use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll, Waker};

use crate::executor::waiter::{TerminationToken, Terminating, interruptible_wait, wake_all};
use crate::memory::PAGE_SIZE_4K;
use crate::memory::vmm::{USER_SPACE_LIMIT, VmOptions, VmaBacking};
use crate::memory::vmo::AnonymousVmo;
use crate::object::handle::HandleTable;
use crate::object::invoke::InvocationError;
use crate::process::{ProcessControlBlock, current_process, register_process};
use crate::sched::dispatch::{create_tcb, enqueue_ready_thread};
use crate::sched::priority::ThreadPriority;
use crate::security::credentials::Credentials;
use crate::time::{get_time, ns_to_ticks, sleep};

static COMPLETE: AtomicBool = AtomicBool::new(false);

extern "C" fn futex_worker(addr: usize) -> ! {
    {
        let process = current_process().expect("futex test process");
        let table = &process.futexes;
        let mut cx = Context::from_waker(Waker::noop());

        let mut invalid = core::pin::pin!(table.wait(USER_SPACE_LIMIT, 0));
        assert_eq!(invalid.as_mut().poll(&mut cx),
            Poll::Ready(Err(InvocationError::InvalidPointer)));

        // First poll resolves the demand fault. The second compares and queues.
        assert!(hal::usercopy::load_user_u32_nofault(addr).is_none());
        let mut first = core::pin::pin!(table.wait(addr, 0));
        assert_eq!(first.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(hal::usercopy::load_user_u32_nofault(addr), Some(0));
        assert_eq!(first.as_mut().poll(&mut cx), Poll::Pending);

        let token = TerminationToken::new();
        let mut cancelled = core::pin::pin!(interruptible_wait(&token, table.wait(addr, 0)));
        assert_eq!(cancelled.as_mut().poll(&mut cx), Poll::Pending);

        let mut last = core::pin::pin!(table.wait(addr, 0));
        assert_eq!(last.as_mut().poll(&mut cx), Poll::Pending);

        wake_all(token.request_and_take_wakers());
        assert_eq!(cancelled.as_mut().poll(&mut cx), Poll::Ready(Err(Terminating)));

        // Select the first waiter before any thread parks. The word stays
        // unchanged, so only an explicitly selected waiter can return.
        table.wake(addr, 1).unwrap();
        assert_eq!(last.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(first.as_mut().poll(&mut cx), Poll::Ready(Ok(())));

        // The cancelled waiter must not consume the next counted wake.
        table.wake(addr, 1).unwrap();
        assert_eq!(last.as_mut().poll(&mut cx), Poll::Ready(Ok(())));
    }

    COMPLETE.store(true, Ordering::Release);
    crate::cpu::current_core_mut().scheduler.terminate_current_thread(0)
}

pub(super) fn run() {
    crate::klogln!(Info, "Test/Futex", "Demand fault, counted wake, and cancellation");
    COMPLETE.store(false, Ordering::Relaxed);
    let process = ProcessControlBlock::new_unregistered(
        HandleTable::new(),
        "futex-diagnostics".into(),
        Credentials::system(),
    ).expect("create futex test process");
    let addr = process.vmm.write().as_mut().unwrap().reserve(
        PAGE_SIZE_4K,
        VmOptions::user_rw(),
        VmaBacking::Vmo(AnonymousVmo::new(PAGE_SIZE_4K)),
    ).expect("reserve demand-paged futex word");

    let worker = create_tcb(
        futex_worker as *const () as usize,
        addr,
        ThreadPriority::HIGH,
        process.clone(),
    ).expect("create futex test worker");
    register_process(&process);
    enqueue_ready_thread(worker);

    let deadline = get_time().saturating_add(ns_to_ticks(5_000_000_000));
    while !COMPLETE.load(Ordering::Acquire) {
        assert!(get_time() < deadline, "futex diagnostics timed out");
        sleep(1_000_000);
    }
}
