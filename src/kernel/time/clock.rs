use core::{
    arch::asm, 
    mem::transmute, 
    sync::atomic::Ordering,
};

use crate::{
    arch::{
        LOCAL_APIC,
        x86_64::interrupts::{
            disable_interrupts, 
            enable_interrupts
        },
    },
    kernel::{
        thread::{
            ThreadState,
            schedule::SCHEDULER,
        },
        time::{
            IA32_TSC_DEADLINE,
            USE_TSC_DEADLINE,
            TIME_SRC_FQ,
            LAPIC_FQ,
            GET_TIME_FN,
            TimeFn,
        },
    },
};


pub fn arm_sleep_ns(ns: usize) {
    if USE_TSC_DEADLINE.load(Ordering::Relaxed) {
        let tsc_fq = TIME_SRC_FQ.load(Ordering::Relaxed);
        let tsc_ticks = (ns * tsc_fq) / 1_000_000_000;

        let mut lo: u32;
        let mut hi: u32;
        unsafe {
            // read tsc
            asm!("rdtsc",
                out("eax") lo, out("edx") hi, options(nomem, nostack));

            let current = ((hi as usize) << 32) | (lo as usize);
            let target = current + tsc_ticks;
            let tgt_lo = (target & 0xFFFF_FFFF) as u32;
            let tgt_hi = (target >> 32) as u32;

            // set deadline
            asm!("wrmsr",
                in("ecx") IA32_TSC_DEADLINE, in("eax") tgt_lo, in("edx") tgt_hi, options(nomem, nostack));
        }
    } else {
        // fallback to lapic
        let lapic_fq = LAPIC_FQ.load(Ordering::Relaxed);
        let lapic_ticks = (ns as usize * lapic_fq) / 1_000_000_000;
        LOCAL_APIC.lock().arm_oneshot(lapic_ticks as u32);
    }
}

pub fn ns_to_ticks(ns: usize) -> usize {
    let freq = TIME_SRC_FQ.load(Ordering::Relaxed);
    ((ns as u128 * freq as u128) / 1_000_000_000) as usize
}

pub fn get_time() -> usize {
    let ptr = GET_TIME_FN.load(Ordering::Relaxed);
    let time_func: TimeFn = unsafe { transmute(ptr) };
    time_func()
}

pub fn sleep(ns: usize) {
    let target_time = get_time() + ns_to_ticks(ns);
    disable_interrupts();

    let mut sched = SCHEDULER.lock();
    let current_thread = sched.get_current_thread();
    unsafe {
        (*current_thread).state = ThreadState::Blocked;
        (*current_thread).wake_time = target_time;
    }
    sched.push_sleep(current_thread);
    sched.schedule();

    drop(sched);
    enable_interrupts();
}

