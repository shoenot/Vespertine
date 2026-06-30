use core::sync::atomic::Ordering;

use hal::interrupts::{
    disable_interrupts,
    enable_interrupts,
};
use vespertine_common::datetime::datetime_to_epoch;

use crate::cpu::current_core_mut;
use crate::core::sync::KernelOnceCell;
use crate::sched::block::ThreadWakeRegistration;
use crate::sched::dispatch::wake_thread;
use crate::sched::priority::ThreadPriority;
use crate::sched::scheduler::ScheduleReason;
use crate::sched::{
    ThreadBlockState,
    ThreadState,
};
use crate::core::time::callout::{
    Callout,
    CalloutPayload,
};

static BOOT_RTC_TIMESTAMP: KernelOnceCell<i64> = KernelOnceCell::new();
static BOOT_TIMESTAMP: KernelOnceCell<i64> = KernelOnceCell::new();

pub fn get_rtc_unix_timestamp() -> i64 {
    datetime_to_epoch(hal::timer::read_rtc())
}

pub fn init_realtime() {
    BOOT_RTC_TIMESTAMP.get_or_init(|| get_rtc_unix_timestamp());
    BOOT_TIMESTAMP.get_or_init(|| get_time() as i64);
}

pub fn get_realtime() -> (i64, i64) {
    let current_time = get_time() as i64;
    let elapsed_ticks = current_time - *BOOT_TIMESTAMP;
    let frequency = hal::timer::counter_frequency() as i64;

    let seconds_passed = elapsed_ticks / frequency;
    let remainder_ticks = elapsed_ticks % frequency;
    let total_seconds = *BOOT_RTC_TIMESTAMP + seconds_passed;
    let nanos = (remainder_ticks * 1_000_000_000) / frequency;

    (total_seconds, nanos)
}

pub fn arm_sleep_ns(ns: usize) {
    hal::timer::arm_relative_ns(ns);
}

pub fn arm_sleep_ticks(ticks: usize) {
    hal::timer::arm_relative_ticks(ticks);
}

pub fn ns_to_ticks(ns: usize) -> usize {
    hal::timer::ns_to_ticks(ns)
}

pub fn get_time() -> usize {
    hal::timer::read_counter()
}

// compares the current quantum and the next callout and sets timer to the earlier of the two.
pub fn update_hardware_timer() {
    let core_data = current_core_mut();
    let current_time = get_time();

    let mut next_event = unsafe {
        if !core_data.scheduler.current_thread.is_null() && (*core_data.scheduler.current_thread).effective_priority != ThreadPriority::IDLE {
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
        arm_sleep_ticks(diff);
    } else if arm_hardware {
        hal::timer::stop();
    }
}

pub fn sleep(ns: usize) {
    let target_time = get_time() + ns_to_ticks(ns);

    disable_interrupts();

    let sched = &mut current_core_mut().scheduler;
    let current_thread = sched.get_current_thread();
    let registration = ThreadWakeRegistration::new(current_thread);

    unsafe {
        (*current_thread).wake_time = target_time;
        (*current_thread).set_block_state(ThreadBlockState::Registration { registration: registration.clone() });
    }

    let callout = Callout { wake_time: target_time, payload: CalloutPayload::WakeThread(registration) };

    {
        let mut queue = current_core_mut().callout_queue.lock();
        queue.push(callout);
        unsafe { (*current_thread).transition(ThreadState::Running, ThreadState::Blocked) }.expect("sleeping thread was not running");
    }

    sched.schedule(ScheduleReason::Blocked);

    enable_interrupts();
}
