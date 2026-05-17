use alloc::alloc::dealloc;
use core::alloc::Layout;
use core::sync::atomic::Ordering;

use crate::arch::x86_64::cpu::fpu::{
    FPU_CXT_SIZE,
    USE_XSAVE,
};
use crate::kernel::thread::ThreadControlBlock;
use crate::kernel::thread::schedule::GRAVEYARD;
use crate::kernel::time::sleep;
use crate::klogln;

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
        // dealloc stack
        let stack_ptr = (*thread).stack_ptr as *mut u8;
        let stack_size = (*thread).stack_size;
        let stack_layout = Layout::from_size_align(stack_size, 16).expect("Error reaping thread");
        dealloc(stack_ptr, stack_layout);

        // dealloc extended context
        let xt_cxt_ptr = (*thread).extended_context;
        let xt_cxt_alignment = if USE_XSAVE.load(Ordering::Relaxed) { 64 } else { 16 };
        let xt_layout = Layout::from_size_align(FPU_CXT_SIZE.load(Ordering::Relaxed), xt_cxt_alignment).expect("Error reaping thread");
        dealloc(xt_cxt_ptr, xt_layout);

        // dealloc tcb
        let tcb_layout = Layout::new::<ThreadControlBlock>();
        dealloc(thread as *mut u8, tcb_layout);
    }
}
