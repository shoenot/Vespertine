use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use crate::arch::hcf;
use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::arch::x86_64::cpu::core::get_core_data;
use crate::kernel::sync::{
    Mutex,
    TicketLock,
};
use crate::kernel::thread::ThreadState;
use crate::kernel::time::sleep;
use crate::klogln;

static SHARED_COUNTER: Mutex<usize> = Mutex::new(0);

#[allow(dead_code)]
pub static RACE_COUNTER: TicketLock<usize> = TicketLock::new(0);
#[allow(dead_code)]
pub static THREADS_FINISHED: AtomicUsize = AtomicUsize::new(0);

pub fn run_demo() -> ! {
    let scheduler = &mut get_core_data().scheduler;

    let tt1 = test_thread as *const ();
    scheduler.spawn(tt1 as usize, 1).unwrap();
    let tt2 = test_thread as *const ();
    scheduler.spawn(tt2 as usize, 2).unwrap();

    scheduler.terminate();
    unreachable!()
}

pub extern "C" fn test_thread(num: usize) -> ! {
    loop {
        klogln!("T{}: attempting to lock...", num);

        {
            let mut guard = SHARED_COUNTER.lock();
            klogln!("T{}: lock acquired! counter is: {}", num, *guard);

            *guard += 1;

            klogln!("T{}: Releasing lock...", num);
        }

        get_core_data().scheduler.schedule();
    }
}

pub extern "C" fn ipi_sniper_thread(_id: usize) -> ! {
    let apic = get_core_data().apic_mode.clone();

    for _ in 0..10 {
        sleep(1_000_000_000);
        klogln!("Core 1: Firing IPIs at sleeping cores");

        apic.send_ipi(0, 64);
        apic.send_ipi(2, 64);
        apic.send_ipi(3, 64);
    }

    loop {
        sleep(1_000_000_000)
    }
}

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

pub fn ap_test_thread(thread_id: usize) -> ! {
    let mut count: usize = 0;
    loop {
        klogln!("This is thread {} on core {} and the counter is at {}", thread_id, get_core_data().lapic_id, count);
        count += 1;
    }
}
