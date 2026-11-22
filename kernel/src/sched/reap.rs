use alloc::alloc::dealloc;
use alloc::sync::Arc;
use hal::context::deallocate_extended_context;
use core::alloc::Layout;
use core::ptr::drop_in_place;
use core::sync::atomic::Ordering;
use crate::sched::Thread;
use crate::sched::scheduler::GRAVEYARD;
use crate::time::sleep;

pub extern "C" fn reaper_daemon(_arg: usize) -> ! {
    loop {
        let budget = GRAVEYARD.lock().queue_length.load(Ordering::Relaxed);
        let mut made_progress = false;

        for _ in 0..budget {
            let zombie = GRAVEYARD.lock().pop();
            if zombie.is_null() { break };

            unsafe {
                let target = &(*zombie).wake_target;
                target.detach();
            }

            if unsafe { (*zombie).reap_pins.load(Ordering::Acquire) } != 0 {
                #[cfg(feature = "diagnostics")]
                let tid = unsafe { (*zombie).thread_id };
                GRAVEYARD.lock().push(zombie);
                #[cfg(feature = "diagnostics")]
                crate::tests::thread_lifetime_tests::note_deferred(tid);
                continue
            }

            reap_thread(zombie);
            made_progress = true;
        }

        if !made_progress {
            sleep(100_000_000);
        }

    }
}

fn reap_thread(thread: *mut Thread) {
    unsafe {
        if let Some(handle) = (*thread).handle.as_ref() {
            handle.detach();
        }
        // bootstrap thread (stack base is 0) so cannot be free by the standard heap
        #[cfg(feature = "diagnostics")]
        let tid = (*thread).thread_id;
        let proc = Arc::clone(&(*thread).process);
        if (*thread).stack_base == 0 {
            drop_in_place(thread);
            proc.finish_thread_reap();
            return;
        }

        let stack_base = (*thread).stack_base as *mut u8;
        let stack_size = (*thread).stack_size;
        let extended_context = (*thread).extended_context;
        let stack_layout = Layout::from_size_align(stack_size, 4096).expect("invalid thread stack layout");
        dealloc(stack_base, stack_layout);

        deallocate_extended_context(extended_context).expect("failed to deallocate thread extended context");
        drop_in_place(thread);

        let tcb_layout = Layout::new::<Thread>();
        dealloc(thread as *mut u8, tcb_layout);
        proc.finish_thread_reap();
        #[cfg(feature = "diagnostics")]
        crate::tests::thread_lifetime_tests::note_freed(tid);
    }
}
