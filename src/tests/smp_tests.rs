use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::time::sleep;
use crate::{
    TicketLock,
    get_core_data,
    klogln,
};

#[allow(dead_code)]
pub fn ap_test_thread(thread_id: usize) -> ! {
    let mut count: usize = 0;
    loop {
        klogln!("This is thread {} on core {} and the counter is at {}", thread_id, get_core_data().lapic_id, count);
        count += 1;
    }
}

#[allow(dead_code)]
pub static RACE_COUNTER: TicketLock<usize> = TicketLock::new(0);
#[allow(dead_code)]
pub static THREADS_FINISHED: AtomicUsize = AtomicUsize::new(0);

#[allow(dead_code)]
pub extern "C" fn contention_thread(_id: usize) -> ! {
    for _ in 0..100_000 {
        let mut guard = RACE_COUNTER.lock();
        let val = *guard;
        *guard = val + 1;
    }

    THREADS_FINISHED.fetch_add(1, Ordering::Relaxed);

    loop {
        crate::kernel::time::sleep(1_000_000);
    }
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
