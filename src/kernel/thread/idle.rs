use core::arch::asm;
use core::ptr::copy_nonoverlapping;
use core::sync::atomic::Ordering;

use crate::arch::x86_64::cpu::fpu::*;
use crate::arch::x86_64::cpu::gdt::{
    KERNEL_CS,
    KERNEL_SS,
};
use crate::arch::x86_64::interrupts::enable_interrupts;
use crate::arch::x86_64::task::context::*;
use crate::kernel::thread::ThreadControlBlock;
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::thread::schedule::RFLAGS_IF;
use crate::{
    BOOTSTRAP_ALLOC,
    klogln,
};

fn idle_loop() -> ! {
    unsafe {
        enable_interrupts();
        loop {
            asm!("sti; hlt", options(nomem, nostack));
        }
    }
}

pub fn init_idle_thread(core_logical_id: usize) -> *mut ThreadControlBlock {
    let stack_size = 4096;

    let tcb_ptr = BOOTSTRAP_ALLOC.lock().alloc(size_of::<ThreadControlBlock>(), 8) as *mut ThreadControlBlock;
    let stack_base = BOOTSTRAP_ALLOC.lock().alloc(stack_size, 8) as usize;

    let fpu_ptr = if USE_XSAVE.load(Ordering::Relaxed) {
        let size = FPU_CXT_SIZE.load(Ordering::Relaxed);
        let fpu_ptr = BOOTSTRAP_ALLOC.lock().alloc(size, 64) as *mut u8;
        let def = CLEAN_FPU_CXT.load(Ordering::Relaxed);
        unsafe { copy_nonoverlapping(def, fpu_ptr, size) };
        fpu_ptr
    } else {
        let fpu_size = FPU_CXT_SIZE.load(Ordering::Relaxed);
        let fpu_ptr = BOOTSTRAP_ALLOC.lock().alloc(fpu_size, 16);
        fpu_ptr as *mut u8
    };

    let stack_top = stack_base + stack_size;
    let context_addr = stack_top - size_of::<ThreadContext>();
    let context_addr = context_addr & !0xF; // align to 16 bytes
    let context = unsafe { &mut *(context_addr as *mut ThreadContext) };

    let idle_loop_addr = idle_loop as *const () as usize;

    context.zero_gp();
    context.instruction_pointer = idle_loop_addr as u64;
    context.stack_pointer = stack_top as u64;
    context.code_segment = KERNEL_CS;
    context.stack_segment = KERNEL_SS;
    context.cpu_flags = RFLAGS_IF;

    let switch_addr = context_addr - size_of::<SwitchContext>();
    let switch_context = unsafe { &mut *(switch_addr as *mut SwitchContext) };

    unsafe extern "C" {
        fn thread_entry_stub();
    }
    let rip = (thread_entry_stub as *const ()) as usize;
    switch_context.init(rip);

    // init TCB
    unsafe {
        (*tcb_ptr).init(switch_addr, stack_base, stack_size, fpu_ptr, core_logical_id, ThreadPriority::IDLE);
        (*tcb_ptr).priority = ThreadPriority::IDLE;
    }

    tcb_ptr
}
