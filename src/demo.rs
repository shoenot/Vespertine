use crate::{
    hcf,
    klogln,
    kernel::{
        SCHEDULER,
        sync::Mutex,
        time::{
            arm_sleep_ns,
            sleep,
        },
    },
};

static SHARED_COUNTER: Mutex<usize> = Mutex::new(0);

pub fn run_demo() -> ! {
    let tt1 = test_thread_1 as *const ();
    let tt2 = test_thread_2 as *const ();


    SCHEDULER.lock().spawn(tt1 as usize).unwrap();
    SCHEDULER.lock().spawn(tt2 as usize).unwrap();

    arm_sleep_ns(10_000_000);

    SCHEDULER.lock().schedule();
    hcf();
}

fn test_thread_1() -> ! {
    loop {
        klogln!("T1: attempting to lock...");

        {
            let mut guard = SHARED_COUNTER.lock();
            klogln!("T1: lock acquired! counter is: {}", *guard);

            *guard += 1;

            klogln!("T1: Releasing lock...");
        }

        SCHEDULER.lock().schedule();

    }
}

fn test_thread_2() -> ! {
    loop {
        klogln!("T2: attempting to lock...");

        {
            let mut guard = SHARED_COUNTER.lock();
            klogln!("T2: lock acquired! counter is: {}", *guard);

            *guard += 1;

            klogln!("T2: Releasing lock...");
        }

        SCHEDULER.lock().schedule();

    }
}
