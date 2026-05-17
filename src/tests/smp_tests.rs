use core::hint::unreachable_unchecked;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::kernel::sync::{
    Mutex,
    Semaphore,
};
use crate::time::sleep;
use crate::{
    TicketLock,
    get_core_data,
    klogln,
    terminate_thread,
};

#[allow(dead_code)]
pub fn ap_test_thread(thread_id: usize) -> ! {
    let mut count: usize = 0;
    loop {
        klogln!("This is thread {} on core {} and the counter is at {}", thread_id, get_core_data().lapic_id, count);
        count += 1;
    }
}

pub static THREADS_FINISHED: AtomicUsize = AtomicUsize::new(0);

pub static MUTEX_RACE: Mutex<usize> = Mutex::new(0);

pub extern "C" fn contention_mutex_thread(_arg: usize) -> ! {
    for _ in 0..100_000 {
        let mut guard = MUTEX_RACE.lock();
        *guard += 1;
        get_core_data().scheduler.schedule();
        drop(guard);
    }
    THREADS_FINISHED.fetch_add(1, Ordering::Relaxed);
    terminate_thread!();
}

pub extern "C" fn ipi_sniper_thread(_id: usize) -> ! {
    let apic = get_core_data().apic_mode.clone();

    for _ in 0..5 {
        sleep(1_000_000_000);
        klogln!("Core 1: Firing IPIs at sleeping cores");

        apic.send_ipi(0, 64);
        apic.send_ipi(2, 64);
        apic.send_ipi(3, 64);
        apic.send_ipi(4, 64);
        apic.send_ipi(5, 64);
        apic.send_ipi(6, 64);
        apic.send_ipi(7, 64);
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
            get_core_data().scheduler.schedule();
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
