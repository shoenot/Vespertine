use core::arch::asm;
use core::mem::transmute;
use core::sync::atomic::Ordering;

use crate::arch::get_rtc_unix_timestamp;
use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::arch::x86_64::cpu::core::get_core_data;
use crate::arch::x86_64::interrupts::{
    disable_interrupts,
    enable_interrupts,
};
use crate::core::sync::KernelOnceCell;
use crate::core::thread::{ThreadBlockState, ThreadState};
use crate::core::thread::dispatch::wake_thread;
use crate::core::thread::priority::ThreadPriority;
use crate::core::thread::schedule::ScheduleReason;
use crate::core::time::callout::{
    Callout,
    CalloutPayload,
};
use crate::core::thread::block::ThreadWakeRegistration;
use crate::core::time::{
    GET_TIME_FN,
    IA32_TSC_DEADLINE,
    LAPIC_FQ,
    TIME_SRC_FQ,
    TimeFn,
    USE_TSC_DEADLINE,
};
use crate::util::write_to_msr;

static BOOT_RTC_TIMESTAMP: KernelOnceCell<i64> = KernelOnceCell::new();
static BOOT_TIMESTAMP: KernelOnceCell<i64> = KernelOnceCell::new();

pub fn init_realtime() {
    BOOT_RTC_TIMESTAMP.get_or_init(|| get_rtc_unix_timestamp());
    BOOT_TIMESTAMP.get_or_init(|| get_time() as i64);
}

pub fn get_realtime() -> (i64, i64) {
    let current_time = get_time() as i64;
    let elapsed_ticks = current_time - *BOOT_TIMESTAMP;
    let tsc_fq = *TIME_SRC_FQ as i64;

    let (seconds_passed, remainder_ticks) = (elapsed_ticks / tsc_fq, elapsed_ticks % tsc_fq);
    let total_seconds = *BOOT_RTC_TIMESTAMP + seconds_passed;
    let nanos = (remainder_ticks * 1_000_000_000) / tsc_fq;

    (total_seconds, nanos)
}

pub fn arm_sleep_ns(ns: usize) {
    if USE_TSC_DEADLINE.load(Ordering::Relaxed) {
        let tsc_fq = *TIME_SRC_FQ;
        let tsc_ticks = (ns * tsc_fq) / 1_000_000_000;

        let mut lo: u32;
        let mut hi: u32;
        unsafe {
            // read tsc
            asm!("rdtsc",
                out("eax") lo, out("edx") hi, options(nomem, nostack));

            let current = ((hi as usize) << 32) | (lo as usize);
            let target = current + tsc_ticks;

            // set deadline
            write_to_msr(target as u64, IA32_TSC_DEADLINE);
        }
    } else {
        let lapic_fq = *LAPIC_FQ;
        let lapic_ticks = (ns as usize * lapic_fq) / 1_000_000_000;

        let core_data = get_core_data();
        core_data.apic_mode.arm_oneshot(lapic_ticks as u32);
    }
}

pub fn arm_sleep_ticks(ticks: usize) {
    if USE_TSC_DEADLINE.load(Ordering::Relaxed) {
        let mut lo: u32;
        let mut hi: u32;
        unsafe {
            // read tsc
            asm!("rdtsc",
                out("eax") lo, out("edx") hi, options(nomem, nostack));

            let current = ((hi as usize) << 32) | (lo as usize);
            let target = current + ticks;

            // set deadline
            write_to_msr(target as u64, IA32_TSC_DEADLINE);
        }
    } else {
        let global_fq = *TIME_SRC_FQ;
        let lapic_fq = *LAPIC_FQ;

        let lapic_ticks = ((ticks as u128 * lapic_fq as u128) / global_fq as u128).max(1);
        let core_data = get_core_data();
        core_data.apic_mode.arm_oneshot(lapic_ticks as u32);
    }
}

pub fn ns_to_ticks(ns: usize) -> usize { ((ns as u128 * *TIME_SRC_FQ as u128) / 1_000_000_000) as usize }

pub fn get_time() -> usize {
    let ptr = GET_TIME_FN.load(Ordering::Relaxed);
    let time_func: TimeFn = unsafe { transmute(ptr) };
    time_func()
}

// compares the current quantum and the next callout and sets timer to the earlier of the two.
pub fn update_hardware_timer() {
    let core_data = get_core_data();
    let current_time = get_time();

    let mut next_event = unsafe {
        if !core_data.scheduler.current_thread.is_null() && (*core_data.scheduler.current_thread).effective_priority != ThreadPriority::IDLE
        {
            (*core_data.scheduler.current_thread).quantum_expiry
        } else {
            usize::MAX
        }
    };

    let mut arm_hardware = true;

    {
        let queue = core_data.callout_queue.lock();
        if let Some(earliest) = queue.peek() {
            if earliest.wake_time < next_event {
                next_event = earliest.wake_time;
            }

            if earliest.wake_time <= current_time {
                arm_hardware = false;
            }
        }
    }

    if !arm_hardware {
        let td_tcb_ptr = core_data.timer_daemon_tcb;
        if !td_tcb_ptr.is_null() {
            core_data.timer_daemon_awoken.store(true, Ordering::Release);
            wake_thread(td_tcb_ptr);
        }
    }

    if arm_hardware && next_event != usize::MAX {
        let diff = next_event.saturating_sub(current_time).max(1);
        let ticks = if diff > u32::MAX as usize { u32::MAX as usize } else { diff };
        arm_sleep_ticks(ticks);
    } else if arm_hardware {
        core_data.apic_mode.stop_timer();
    }
}

pub fn sleep(ns: usize) {
    let target_time = get_time() + ns_to_ticks(ns);

    disable_interrupts();

    let core_data = get_core_data();
    let sched = &mut core_data.scheduler;
    let current_thread = sched.get_current_thread();
    let registration = ThreadWakeRegistration::new(current_thread);

    unsafe {
        (*current_thread).wake_time = target_time;
        (*current_thread).set_block_state(ThreadBlockState::Registration { registration: registration.clone() });
    }

    let callout = Callout { wake_time: target_time, payload: CalloutPayload::WakeThread(registration) };

    {
        let mut queue = get_core_data().callout_queue.lock();
        queue.push(callout);
        unsafe { (*current_thread).transition(ThreadState::Running, ThreadState::Blocked) }.expect("sleeping thread was not running");
    }

    sched.schedule(ScheduleReason::Blocked);

    enable_interrupts();
}
