use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use hal::ipi::send_reschedule_ipi;
use crate::core::sync::{
    Mutex,
    Semaphore,
};
use crate::sched::scheduler::ScheduleReason;
use crate::time::sleep;
use crate::{
    klogln,
    terminate_thread,
};
use crate::cpu::{current_core_id, current_core_mut};

#[allow(dead_code)]
pub fn ap_test_thread(thread_id: usize) -> ! {
    let mut count: usize = 0;
    loop {
        klogln!("This is thread {} on core {} and the counter is at {}", thread_id, current_core_id(), count);
        count += 1;
    }
}

pub static THREADS_FINISHED: AtomicUsize = AtomicUsize::new(0);

pub static MUTEX_RACE: Mutex<usize> = Mutex::new(0);

pub extern "C" fn contention_mutex_thread(_arg: usize) -> ! {
    for _ in 0..100_000 {
        let mut guard = MUTEX_RACE.lock();
        *guard += 1;
        current_core_mut().scheduler.schedule(ScheduleReason::Yield);
        drop(guard);
    }
    THREADS_FINISHED.fetch_add(1, Ordering::Relaxed);
    terminate_thread!();
}

pub extern "C" fn ipi_sniper_thread(_id: usize) -> ! {
    for _ in 0..5 {
        sleep(1_000_000_000);
        klogln!("Core 1: Firing IPIs at sleeping cores");

        send_reschedule_ipi(0);
        send_reschedule_ipi(2);
        send_reschedule_ipi(3);
        send_reschedule_ipi(4);
        send_reschedule_ipi(5);
        send_reschedule_ipi(6);
        send_reschedule_ipi(7);
    }

    loop {
        sleep(1_000_000_000)
    }
}

const BUFFER_SIZE: usize = 16;
pub static mut PRODUCER_BUFFER: [usize; BUFFER_SIZE] = [0; BUFFER_SIZE];
pub static PRODUCER_TAIL: AtomicUsize = AtomicUsize::new(0);
pub static CONSUMER_HEAD: AtomicUsize = AtomicUsize::new(0);

// The two semaphores that control the flow
pub static SLOTS_AVAILABLE: Semaphore = Semaphore::new(BUFFER_SIZE as isize);
pub static ITEMS_READY: Semaphore = Semaphore::new(0);

pub static PRODUCER_THREADS_FINISHED: AtomicUsize = AtomicUsize::new(0);

pub extern "C" fn producer_thread(_arg: usize) -> ! {
    for _ in 0..25_000 {
        SLOTS_AVAILABLE.wait();

        let tail = PRODUCER_TAIL.fetch_add(1, Ordering::Relaxed) % BUFFER_SIZE;
        unsafe {
            PRODUCER_BUFFER[tail] = 1;
        }

        if tail % 4 == 0 {
            current_core_mut().scheduler.schedule(ScheduleReason::Yield);
        }

        ITEMS_READY.signal();
    }
    PRODUCER_THREADS_FINISHED.fetch_add(1, Ordering::Relaxed);
    terminate_thread!();
}

pub extern "C" fn consumer_thread(expected_total: usize) -> ! {
    let mut items_consumed = 0;

    for _ in 0..expected_total {
        ITEMS_READY.wait();

        let head = CONSUMER_HEAD.fetch_add(1, Ordering::Relaxed) % BUFFER_SIZE;
        let _val = unsafe { PRODUCER_BUFFER[head] };

        SLOTS_AVAILABLE.signal();

        items_consumed += 1;
    }

    klogln!("Consumer successfully processed {} items.", items_consumed);
    THREADS_FINISHED.fetch_add(1, Ordering::Relaxed);
    terminate_thread!();
}
