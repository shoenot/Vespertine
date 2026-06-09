use core::sync::atomic::{
    AtomicBool,
    AtomicPtr,
    AtomicUsize,
    Ordering,
};

use crate::arch::get_core_data;
use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::core::cpu::{
    NUM_CORES,
    get_core_data_for,
};
use crate::core::thread::ThreadState;
use crate::core::thread::dispatch::{
    cancel_block_if_awoken,
    create_tcb,
    try_wake_thread,
    wake_thread,
};
use crate::core::thread::priority::ThreadPriority;
use crate::{
    KERNEL_PROCESS,
    terminate_thread,
};

fn current_thread() -> &'static crate::core::thread::ThreadControlBlock { unsafe { &*get_core_data().scheduler.get_current_thread() } }

static WAKE_TARGET: AtomicPtr<crate::core::thread::ThreadControlBlock> = AtomicPtr::new(core::ptr::null_mut());
static START_WAKE_RACE: AtomicBool = AtomicBool::new(false);
static WAKE_WORKERS_DONE: AtomicUsize = AtomicUsize::new(0);
static WAKE_TARGET_RUNS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn wake_race_worker(_arg: usize) -> ! {
    while !START_WAKE_RACE.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }

    wake_thread(WAKE_TARGET.load(Ordering::Acquire));
    WAKE_WORKERS_DONE.fetch_add(1, Ordering::Release);
    terminate_thread!();
}

extern "C" fn wake_race_target(_arg: usize) -> ! {
    WAKE_TARGET_RUNS.fetch_add(1, Ordering::Release);
    terminate_thread!();
}

fn enqueue_test_thread_on_core(thread: *mut crate::core::thread::ThreadControlBlock, core: usize) {
    unsafe {
        (*thread).home_core = core;
    }
    let target_data = get_core_data_for(core);
    target_data.scheduler.mailbox.lock().push(thread);

    if core != get_core_data().logical_id {
        get_core_data().apic_mode.send_ipi(target_data.lapic_id as u32, 40);
    }
}

fn test_two_cpu_wakes_enqueue_blocked_thread_once() {
    assert!(*NUM_CORES >= 2, "two-CPU wake test requires at least two CPUs");
    START_WAKE_RACE.store(false, Ordering::Release);
    WAKE_WORKERS_DONE.store(0, Ordering::Release);
    WAKE_TARGET_RUNS.store(0, Ordering::Release);

    let target = create_tcb(wake_race_target as *const () as usize, 0, ThreadPriority::HIGH, KERNEL_PROCESS.clone())
        .expect("failed to create wake target");
    unsafe {
        (*target).set_state(ThreadState::Blocked);
        (*target).home_core = get_core_data().logical_id;
    }
    WAKE_TARGET.store(target, Ordering::Release);

    for core in 0..2 {
        let worker = create_tcb(wake_race_worker as *const () as usize, 0, ThreadPriority::HIGH, KERNEL_PROCESS.clone())
            .expect("failed to create wake worker");
        enqueue_test_thread_on_core(worker, core);
    }
    START_WAKE_RACE.store(true, Ordering::Release);

    for _ in 0..10_000 {
        if WAKE_WORKERS_DONE.load(Ordering::Acquire) == 2 && WAKE_TARGET_RUNS.load(Ordering::Acquire) == 1 {
            break;
        }
        get_core_data().scheduler.schedule();
    }

    assert_eq!(WAKE_WORKERS_DONE.load(Ordering::Acquire), 2, "wake race workers did not finish");
    assert_eq!(WAKE_TARGET_RUNS.load(Ordering::Acquire), 1, "blocked target was not enqueued exactly once");
}

fn test_wake_immediately_before_blocking_cancels_block() {
    let thread = current_thread();
    let awoken = AtomicBool::new(true);

    thread.transition(ThreadState::Running, ThreadState::Blocked).expect("test thread was not running");
    assert!(cancel_block_if_awoken(thread, &awoken), "wake-before-block did not cancel blocking");
    assert_eq!(thread.state(), ThreadState::Running);
}

fn test_wake_immediately_after_blocking_makes_thread_ready() {
    let thread = current_thread();
    let awoken = AtomicBool::new(false);

    thread.transition(ThreadState::Running, ThreadState::Blocked).expect("test thread was not running");
    awoken.store(true, Ordering::Release);
    assert!(try_wake_thread(thread as *const _ as *mut _), "wake-after-block did not claim thread");
    assert!(!cancel_block_if_awoken(thread, &awoken), "ready thread was incorrectly restored directly to running");
    assert_eq!(thread.state(), ThreadState::Ready);
    thread.transition(ThreadState::Ready, ThreadState::Running).expect("test thread was not ready");
}

pub fn run_concurrency_tests() {
    crate::klogln!("[TEST] atomic wake claim");
    test_two_cpu_wakes_enqueue_blocked_thread_once();
    crate::klogln!("[TEST] wake before block");
    test_wake_immediately_before_blocking_cancels_block();
    crate::klogln!("[TEST] wake after block");
    test_wake_immediately_after_blocking_makes_thread_ready();
    crate::core::asynchronous::run_diagnostic_tests();
    crate::core::object::models::socket::run_diagnostic_tests();
    crate::drivers::virtio::blk::run_diagnostic_tests();
    crate::klogln!("[TEST] concurrency invariants passed");
}
