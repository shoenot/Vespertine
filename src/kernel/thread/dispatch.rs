use alloc::alloc::alloc;
use core::alloc::Layout;
use core::ptr::{
    copy_nonoverlapping,
    write_volatile,
};
use core::sync::atomic::Ordering;

use crate::arch::get_core_data;
use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::arch::x86_64::cpu::fpu::{
    CLEAN_LEGACY_FPU_CXT,
    FPU_CXT_SIZE,
    LegacyXtCxt,
    USE_XSAVE,
    gen_avx_dummy_fpu,
};
use crate::arch::x86_64::task::context::{
    SwitchContext,
    ThreadContext,
};
use crate::kernel::cpu::{
    NUM_CORES,
    try_get_core_data_for,
    get_core_data_for,
};
use crate::kernel::thread::priority::ThreadPriority;
use crate::kernel::thread::{
    ThreadControlBlock,
    ThreadError,
    ThreadState,
};

pub fn spawn_kernel_thread(entry_point: usize, arg: usize, priority: ThreadPriority) -> *mut ThreadControlBlock {
    let tcb_ptr = create_tcb(entry_point, arg, priority).expect("Unable to spawn kernel thread");

    let mut best_core = 0;
    let mut min_load = usize::MAX;

    for logical_id in 0..*NUM_CORES {
        if let Some(target_data) = try_get_core_data_for(logical_id) {
            let load = target_data.scheduler.queue_length.load(Ordering::Acquire) +
                target_data.scheduler.mailbox.lock().queue_length.load(Ordering::Acquire);
            if load < min_load {
                min_load = load;
                best_core = logical_id;
            }
        }
    }

    unsafe {
        (*tcb_ptr).home_core = best_core;
    }

    let this_core = get_core_data().logical_id;
    let target_data = get_core_data_for(best_core);

    if best_core == this_core {
        target_data.scheduler.mailbox.lock().push(tcb_ptr);
    } else {
        let mut mailbox = target_data.scheduler.mailbox.lock();
        mailbox.push(tcb_ptr);
        drop(mailbox);

        get_core_data().apic_mode.send_ipi(target_data.lapic_id as u32, 64);
    }
    tcb_ptr
}

pub fn wake_thread(thread: *mut ThreadControlBlock) {
    unsafe {
        (*thread).state = ThreadState::Ready;

        let this_core = get_core_data().logical_id;
        let target_core = (*thread).home_core;

        if this_core == target_core {
            // local thread wakeup
            get_core_data().scheduler.push(thread);
        } else {
            let target_data = get_core_data_for(target_core);

            let mut mailbox = target_data.scheduler.mailbox.lock();
            mailbox.push(thread);
            drop(mailbox);

            get_core_data().apic_mode.send_ipi(target_data.lapic_id as u32, 64);
        }
    }
}

pub fn create_tcb(entry_point: usize, arg: usize, priority: ThreadPriority) -> Result<*mut ThreadControlBlock, ThreadError> {
    let stack_size = 4096 * 4;
    let fpu_size = FPU_CXT_SIZE.load(Ordering::Relaxed);
    // alloc memory for structs
    let tcb_layout = Layout::new::<ThreadControlBlock>();
    let stack_layout = Layout::from_size_align(stack_size, 4096)?;

    let tcb_ptr = unsafe { alloc(tcb_layout) as *mut ThreadControlBlock };
    let stack_base = unsafe { alloc(stack_layout) as usize };

    unsafe {
        let stack_ptr_u64 = stack_base as *mut u64;
        for i in 0..(stack_size / 8) {
            write_volatile(stack_ptr_u64.add(i), 0);
        }

        write_volatile(tcb_ptr as *mut u8, 0);
    }

    // init extended context state
    let fpu_ptr = if USE_XSAVE.load(Ordering::Relaxed) {
        gen_avx_dummy_fpu()?
    } else {
        let fpu_layout = Layout::from_size_align(fpu_size, 16)?;
        let fpu_ptr = unsafe { alloc(fpu_layout) as *mut u8 };
        let def = CLEAN_LEGACY_FPU_CXT.lock();
        let default_fpu_ref = def.as_ref().expect("Clean FPU not initialized");
        unsafe { copy_nonoverlapping(default_fpu_ref as *const LegacyXtCxt, fpu_ptr as *mut LegacyXtCxt, 1) };
        fpu_ptr as *mut u8
    };

    let stack_top = stack_base + stack_size;
    let context_addr = stack_top - size_of::<ThreadContext>();
    let context_addr = context_addr & !0xF; // align to 16 bytes
    let context = unsafe { &mut *(context_addr as *mut ThreadContext) };

    context.init(entry_point as u64, (stack_top - 8) as u64, arg);

    let switch_addr = context_addr - size_of::<SwitchContext>();
    let switch_context = unsafe { &mut *(switch_addr as *mut SwitchContext) };

    unsafe extern "C" {
        fn thread_entry_stub();
    }
    switch_context.init((thread_entry_stub as *const ()) as usize);

    // init TCB
    unsafe {
        (*tcb_ptr).init(switch_addr, stack_base, stack_size, fpu_ptr, 0, priority);
    }

    Ok(tcb_ptr)
}
