use alloc::alloc::dealloc;
use hal::context::deallocate_extended_context;
use core::alloc::Layout;
use core::ptr::drop_in_place;
use crate::core::thread::ThreadControlBlock;
use crate::core::thread::schedule::GRAVEYARD;
use crate::core::time::sleep;

pub extern "C" fn reaper_daemon(_arg: usize) -> ! {
    loop {
        let mut graveyard = GRAVEYARD.lock();
        let zombie = graveyard.pop();
        drop(graveyard);

        if !zombie.is_null() {
            reap_thread(zombie);
        } else {
            sleep(100_000_000);
        }
    }
}

fn reap_thread(thread: *mut ThreadControlBlock) {
    unsafe {
        // bootstrap thread (stack base is 0) so cannot be free by the standard heap
        if (*thread).stack_base == 0 {
            drop_in_place(thread);
            return;
        }

        let stack_base = (*thread).stack_base as *mut u8;
        let stack_size = (*thread).stack_size;
        let extended_context = (*thread).extended_context;
        let stack_layout = Layout::from_size_align(stack_size, 4096).expect("invalid thread stack layout");
        dealloc(stack_base, stack_layout);

        deallocate_extended_context(extended_context).expect("failed to deallocate thread extended context");
        drop_in_place(thread);

        let tcb_layout = Layout::new::<ThreadControlBlock>();
        dealloc(thread as *mut u8, tcb_layout);
    }
}
