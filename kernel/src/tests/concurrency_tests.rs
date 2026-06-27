use core::sync::atomic::{
    AtomicBool,
    AtomicPtr,
    AtomicUsize,
    Ordering,
};

use hal::arch::interrupts::{
    disable_interrupts,
    enable_interrupts,
    interrupts_enabled,
};

use crate::arch::get_core_data;
use crate::arch::x86_64::apic::lapic::ApicDriver;
use crate::core::cpu::{
    NO_STEAL_REQUEST,
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
use crate::core::thread::schedule::{
    ScheduleReason,
    SchedulerState,
    account_running_thread,
    should_refresh_quantum,
};
use crate::{
    KERNEL_PROCESS,
    terminate_thread,
};

fn current_thread() -> &'static crate::core::thread::ThreadControlBlock { unsafe { &*get_core_data().scheduler.get_current_thread() } }

fn test_yield() {
    let restore_interrupts = interrupts_enabled();
    get_core_data().scheduler.schedule(ScheduleReason::Yield);
    if restore_interrupts {
        enable_interrupts();
    }
}

static WAKE_TARGET: AtomicPtr<crate::core::thread::ThreadControlBlock> = AtomicPtr::new(core::ptr::null_mut());
static START_WAKE_RACE: AtomicBool = AtomicBool::new(false);
static WAKE_WORKERS_DONE: AtomicUsize = AtomicUsize::new(0);
static WAKE_TARGET_RUNS: AtomicUsize = AtomicUsize::new(0);
static STEAL_WORKERS_DONE: AtomicUsize = AtomicUsize::new(0);
static STEAL_REMOTE_RUNS: AtomicUsize = AtomicUsize::new(0);
static MIGRATED_BLOCKED_THREAD: AtomicPtr<crate::core::thread::ThreadControlBlock> = AtomicPtr::new(core::ptr::null_mut());
static MIGRATED_FIRST_CORE: AtomicUsize = AtomicUsize::new(usize::MAX);
static MIGRATED_SECOND_CORE: AtomicUsize = AtomicUsize::new(usize::MAX);

const SMP_TEST_WAIT_ITERATIONS: usize = 1_000_000;

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

extern "C" fn steal_worker(owner_core: usize) -> ! {
    if get_core_data().logical_id != owner_core {
        STEAL_REMOTE_RUNS.fetch_add(1, Ordering::Release);
    }
    STEAL_WORKERS_DONE.fetch_add(1, Ordering::Release);
    terminate_thread!();
}

extern "C" fn migrated_wake_target(_arg: usize) -> ! {
    let core_data = get_core_data();
    let thread = core_data.scheduler.get_current_thread();
    MIGRATED_FIRST_CORE.store(core_data.logical_id, Ordering::Release);
    unsafe {
        (*thread).transition(ThreadState::Running, ThreadState::Blocked).expect("migrated wake target was not running");
    }
    MIGRATED_BLOCKED_THREAD.store(thread, Ordering::Release);
    core_data.scheduler.schedule(ScheduleReason::Blocked);
    MIGRATED_SECOND_CORE.store(get_core_data().logical_id, Ordering::Release);
    terminate_thread!();
}

fn enqueue_test_thread_on_core(thread: *mut crate::core::thread::ThreadControlBlock, core: usize) {
    unsafe {
        (*thread).set_assigned_core(core);
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
        (*target).set_assigned_core(get_core_data().logical_id);
    }
    WAKE_TARGET.store(target, Ordering::Release);

    for core in 0..2 {
        let worker = create_tcb(wake_race_worker as *const () as usize, 0, ThreadPriority::HIGH, KERNEL_PROCESS.clone())
            .expect("failed to create wake worker");
        enqueue_test_thread_on_core(worker, core);
    }
    START_WAKE_RACE.store(true, Ordering::Release);

    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if WAKE_WORKERS_DONE.load(Ordering::Acquire) == 2 && WAKE_TARGET_RUNS.load(Ordering::Acquire) == 1 {
            break;
        }
        test_yield();
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
    unsafe {
        (*(thread as *const _ as *mut crate::core::thread::ThreadControlBlock)).effective_priority = thread.base_priority;
    }
}

fn test_early_timer_event_preserves_quantum() {
    let thread = get_core_data().scheduler.get_current_thread();
    let now = 100;
    let expiry = 200;

    assert!(!should_refresh_quantum(thread, thread, ScheduleReason::TimerEvent, expiry, now));
    assert!(!should_refresh_quantum(thread, thread, ScheduleReason::RescheduleIpi, expiry, now));
    assert!(should_refresh_quantum(thread, thread, ScheduleReason::QuantumExpired, expiry, now));
}

fn test_priority_from_clamps_invalid_values() {
    assert_eq!(ThreadPriority::from(31), ThreadPriority::IDLE);
    assert_eq!(ThreadPriority::from(32), ThreadPriority::IDLE);
    assert_eq!(ThreadPriority::from(u8::MAX), ThreadPriority::IDLE);
}

fn test_woken_thread_receives_boost() {
    let thread = create_tcb(wake_race_target as *const () as usize, 0, ThreadPriority::MEDIUM, KERNEL_PROCESS.clone())
        .expect("failed to create boost target");
    unsafe {
        (*thread).set_state(ThreadState::Blocked);
    }

    assert!(try_wake_thread(thread));
    unsafe {
        assert_eq!((*thread).effective_priority, ThreadPriority::MEDIUM.boosted(2));
    }
}

fn test_boost_decays_once_per_completed_quantum() {
    let thread = create_tcb(wake_race_target as *const () as usize, 0, ThreadPriority::MEDIUM, KERNEL_PROCESS.clone())
        .expect("failed to create decay target");
    unsafe {
        (*thread).effective_priority = (*thread).base_priority.boosted(2);
        (*thread).last_started = 10;

        account_running_thread(&mut *thread, ScheduleReason::TimerEvent, 20);
        assert_eq!((*thread).effective_priority, ThreadPriority::MEDIUM.boosted(2));

        account_running_thread(&mut *thread, ScheduleReason::QuantumExpired, 30);
        assert_eq!((*thread).effective_priority, ThreadPriority::MEDIUM.boosted(1));
    }
}

fn test_migration_disabled_thread_is_never_donated() {
    let core = get_core_data().logical_id;
    let pinned = create_tcb(wake_race_target as *const () as usize, 0, ThreadPriority::LOW, KERNEL_PROCESS.clone())
        .expect("failed to create pinned target");
    let migratable = create_tcb(wake_race_target as *const () as usize, 0, ThreadPriority::MEDIUM, KERNEL_PROCESS.clone())
        .expect("failed to create migratable target");
    unsafe {
        (*pinned).pin_to_core(core);
        (*migratable).set_assigned_core(core);
    }

    let mut scheduler = SchedulerState::new();
    scheduler.init_basic(core);
    scheduler.push(pinned);
    scheduler.push(migratable);

    assert_eq!(scheduler.pop_lowest_priority_migratable(), migratable);
    assert_eq!(scheduler.pop(), pinned);
}

fn test_idle_cpu_obtains_migratable_work() {
    assert!(*NUM_CORES >= 2, "work-stealing test requires at least two CPUs");
    let owner = get_core_data().logical_id;
    STEAL_WORKERS_DONE.store(0, Ordering::Release);
    STEAL_REMOTE_RUNS.store(0, Ordering::Release);
    disable_interrupts();

    for _ in 0..2 {
        let worker = create_tcb(steal_worker as *const () as usize, owner, ThreadPriority::LOW, KERNEL_PROCESS.clone())
            .expect("failed to create steal worker");
        unsafe {
            (*worker).set_assigned_core(owner);
        }
        get_core_data().scheduler.push(worker);
    }

    let target = get_core_data_for(1);
    get_core_data().apic_mode.send_ipi(target.lapic_id as u32, 40);

    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if get_core_data().steal_requester.load(Ordering::Acquire) != NO_STEAL_REQUEST {
            break;
        }
        core::hint::spin_loop();
    }
    assert_ne!(get_core_data().steal_requester.load(Ordering::Acquire), NO_STEAL_REQUEST, "idle CPU did not request work");

    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if STEAL_WORKERS_DONE.load(Ordering::Acquire) == 2 {
            break;
        }
        test_yield();
    }
    enable_interrupts();

    assert_eq!(STEAL_WORKERS_DONE.load(Ordering::Acquire), 2, "steal workers did not finish");
    assert!(STEAL_REMOTE_RUNS.load(Ordering::Acquire) >= 1, "idle CPU did not obtain migratable work");
}

fn test_stolen_thread_wakeup_routes_to_assigned_cpu() {
    assert!(*NUM_CORES >= 2, "wake routing test requires at least two CPUs");
    let owner = get_core_data().logical_id;
    MIGRATED_BLOCKED_THREAD.store(core::ptr::null_mut(), Ordering::Release);
    MIGRATED_FIRST_CORE.store(usize::MAX, Ordering::Release);
    MIGRATED_SECOND_CORE.store(usize::MAX, Ordering::Release);
    disable_interrupts();

    let target = create_tcb(migrated_wake_target as *const () as usize, 0, ThreadPriority::LOW, KERNEL_PROCESS.clone())
        .expect("failed to create routed wake target");
    let filler = create_tcb(steal_worker as *const () as usize, owner, ThreadPriority::LOW, KERNEL_PROCESS.clone())
        .expect("failed to create wake-routing filler");
    unsafe {
        (*target).set_assigned_core(owner);
        (*filler).set_assigned_core(owner);
    }
    get_core_data().scheduler.push(target);
    get_core_data().scheduler.push(filler);

    let remote = get_core_data_for(1);
    get_core_data().apic_mode.send_ipi(remote.lapic_id as u32, 40);

    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if get_core_data().steal_requester.load(Ordering::Acquire) != NO_STEAL_REQUEST {
            break;
        }
        core::hint::spin_loop();
    }
    assert_ne!(get_core_data().steal_requester.load(Ordering::Acquire), NO_STEAL_REQUEST, "idle CPU did not request routed work");

    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        let blocked = MIGRATED_BLOCKED_THREAD.load(Ordering::Acquire);
        if !blocked.is_null() {
            wake_thread(blocked);
            break;
        }
        test_yield();
    }
    enable_interrupts();

    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if MIGRATED_SECOND_CORE.load(Ordering::Acquire) != usize::MAX {
            break;
        }
        test_yield();
    }

    let first_core = MIGRATED_FIRST_CORE.load(Ordering::Acquire);
    assert_ne!(first_core, owner, "wake target was not stolen");
    assert_eq!(MIGRATED_SECOND_CORE.load(Ordering::Acquire), first_core, "stolen thread woke on the wrong assigned CPU");
}

pub fn run_concurrency_tests() {
    crate::klogln!("[TEST] early timer preserves quantum");
    test_early_timer_event_preserves_quantum();
    crate::klogln!("[TEST] priority validation");
    test_priority_from_clamps_invalid_values();
    crate::klogln!("[TEST] wake boost");
    test_woken_thread_receives_boost();
    crate::klogln!("[TEST] completed-quantum boost decay");
    test_boost_decays_once_per_completed_quantum();
    crate::klogln!("[TEST] atomic wake claim");
    test_two_cpu_wakes_enqueue_blocked_thread_once();
    crate::klogln!("[TEST] idle CPU steals migratable work");
    test_idle_cpu_obtains_migratable_work();
    crate::klogln!("[TEST] pinned thread is not donated");
    test_migration_disabled_thread_is_never_donated();
    crate::klogln!("[TEST] migrated wake routing");
    test_stolen_thread_wakeup_routes_to_assigned_cpu();
    crate::klogln!("[TEST] wake before block");
    test_wake_immediately_before_blocking_cancels_block();
    crate::klogln!("[TEST] wake after block");
    test_wake_immediately_after_blocking_makes_thread_ready();
    crate::core::asynchronous::run_diagnostic_tests();
    crate::core::object::models::socket::run_diagnostic_tests();
    crate::drivers::virtio::blk::run_diagnostic_tests();
    crate::klogln!("[TEST] concurrency invariants passed");
}
