use core::sync::atomic::{
    AtomicBool,
    AtomicPtr,
    AtomicUsize,
    Ordering,
};

use hal::interrupts::{
    disable_interrupts,
    enable_interrupts,
    interrupts_enabled,
};

use hal::ipi::send_reschedule_ipi;
use crate::cpu::{
    NO_STEAL_REQUEST, NUM_CORES, current_core_id, current_core_mut, get_core_data_for
};
use crate::sched::ThreadState;
use crate::sched::dispatch::{
    cancel_block_if_awoken,
    create_tcb,
    try_wake_thread,
    wake_thread,
};
use crate::sched::priority::ThreadPriority;
use crate::sched::scheduler::{
    ScheduleReason,
    SchedulerState,
    account_running_thread,
    should_refresh_quantum,
};
use crate::{
    KERNEL_PROCESS,
    terminate_thread,
};

fn current_thread() -> &'static crate::sched::Thread { unsafe { &*current_core_mut().scheduler.get_current_thread() } }

fn test_yield() {
    let restore_interrupts = interrupts_enabled();
    current_core_mut().scheduler.schedule(ScheduleReason::Yield);
    if restore_interrupts {
        enable_interrupts();
    }
}

static WAKE_TARGET: AtomicPtr<crate::sched::Thread> = AtomicPtr::new(core::ptr::null_mut());
static START_WAKE_RACE: AtomicBool = AtomicBool::new(false);
static WAKE_WORKERS_DONE: AtomicUsize = AtomicUsize::new(0);
static WAKE_TARGET_RUNS: AtomicUsize = AtomicUsize::new(0);
static STEAL_WORKERS_DONE: AtomicUsize = AtomicUsize::new(0);
static STEAL_REMOTE_RUNS: AtomicUsize = AtomicUsize::new(0);
static MIGRATED_BLOCKED_THREAD: AtomicPtr<crate::sched::Thread> = AtomicPtr::new(core::ptr::null_mut());
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
    if current_core_id() != owner_core {
        STEAL_REMOTE_RUNS.fetch_add(1, Ordering::Release);
    }
    STEAL_WORKERS_DONE.fetch_add(1, Ordering::Release);
    terminate_thread!();
}

extern "C" fn migrated_wake_target(_arg: usize) -> ! {
    let core_data = current_core_mut();
    let thread = core_data.scheduler.get_current_thread();
    MIGRATED_FIRST_CORE.store(core_data.logical_id, Ordering::Release);
    unsafe {
        (*thread).transition(ThreadState::Running, ThreadState::Blocked).expect("migrated wake target was not running");
    }
    MIGRATED_BLOCKED_THREAD.store(thread, Ordering::Release);
    core_data.scheduler.schedule(ScheduleReason::Blocked);
    MIGRATED_SECOND_CORE.store(current_core_id(), Ordering::Release);
    terminate_thread!();
}

fn enqueue_test_thread_on_core(thread: *mut crate::sched::Thread, core: usize) {
    unsafe {
        (*thread).set_assigned_core(core);
    }
    let target_data = get_core_data_for(core);
    target_data.scheduler.mailbox.lock().push(thread);

    if core != current_core_id() {
        send_reschedule_ipi(core);
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
        (*target).set_assigned_core(current_core_id());
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
        (*(thread as *const _ as *mut crate::sched::Thread)).effective_priority = thread.base_priority;
    }
}

fn test_early_timer_event_preserves_quantum() {
    let thread = current_core_mut().scheduler.get_current_thread();
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
    let core = current_core_id();
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
    let owner = current_core_id();
    STEAL_WORKERS_DONE.store(0, Ordering::Release);
    STEAL_REMOTE_RUNS.store(0, Ordering::Release);
    disable_interrupts();

    for _ in 0..2 {
        let worker = create_tcb(steal_worker as *const () as usize, owner, ThreadPriority::LOW, KERNEL_PROCESS.clone())
            .expect("failed to create steal worker");
        unsafe {
            (*worker).set_assigned_core(owner);
        }
        current_core_mut().scheduler.push(worker);
    }

    let target = get_core_data_for(1);
    send_reschedule_ipi(target.logical_id);

    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if current_core_mut().steal_requester.load(Ordering::Acquire) != NO_STEAL_REQUEST {
            break;
        }
        core::hint::spin_loop();
    }
    assert_ne!(current_core_mut().steal_requester.load(Ordering::Acquire), NO_STEAL_REQUEST, "idle CPU did not request work");

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
    let owner = current_core_id();
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
    current_core_mut().scheduler.push(target);
    current_core_mut().scheduler.push(filler);

    let remote = get_core_data_for(1);
    send_reschedule_ipi(remote.logical_id);

    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if current_core_mut().steal_requester.load(Ordering::Acquire) != NO_STEAL_REQUEST {
            break;
        }
        core::hint::spin_loop();
    }
    assert_ne!(current_core_mut().steal_requester.load(Ordering::Acquire), NO_STEAL_REQUEST, "idle CPU did not request routed work");

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

fn test_registered_wake_before_park() {
    use alloc::sync::Arc;
    use crate::sched::block::ThreadWakeRegistration;
    use crate::sched::dispatch::resume_registered_if_awoken;

    let restore = interrupts_enabled();
    disable_interrupts();
    let target = current_thread();
    let registration = ThreadWakeRegistration::new(Arc::clone(&target.wake_target));
    // ThreadWaker sets its latch before firing the registration.
    let awoken = AtomicBool::new(true);
    assert!(registration.wake());
    target.park_registration(Arc::clone(&registration));
    assert!(resume_registered_if_awoken(target, &registration, &awoken));
    assert_eq!(target.state(), ThreadState::Running);
    if restore { enable_interrupts(); }
}

static REGISTERED_TARGET_RUNS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn registered_target(_arg: usize) -> ! {
    REGISTERED_TARGET_RUNS.fetch_add(1, Ordering::Release);
    terminate_thread!();
}

fn test_active_invocation_retains_inner_park() {
    use alloc::sync::Arc;
    use crate::sched::block::ThreadWakeRegistration;

    REGISTERED_TARGET_RUNS.store(0, Ordering::Release);
    let ptr = create_tcb(registered_target as *const () as usize, 0,
        ThreadPriority::HIGH, KERNEL_PROCESS.clone()).expect("create park target");
    // The target has never been enqueued. Its first dispatch represents
    // resumption of the protected invocation after its inner wait completes.
    let (outer, inner) = unsafe {
        let target = &*ptr;
        target.set_assigned_core(current_core_id());
        target.set_state(ThreadState::Running);
        target.user_kernel_active.store(true, Ordering::Release);
        let outer = ThreadWakeRegistration::new(Arc::clone(&target.wake_target));
        let inner = ThreadWakeRegistration::new(Arc::clone(&target.wake_target));
        target.park_registration(Arc::clone(&inner));

        assert!(outer.wake());
        assert!(outer.fired());
        target.request_cancel();
        assert!(target.cancel_requested());
        assert_eq!(target.state(), ThreadState::Blocked);
        assert!(inner.is_active());
        assert!(!try_wake_thread(ptr));
        assert!(target.block_state.lock().as_ref()
            .is_some_and(|installed| Arc::ptr_eq(installed, &inner)));
        (outer, inner)
    };

    assert!(!outer.wake());
    assert!(inner.wake());
    assert!(!inner.wake());
    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if REGISTERED_TARGET_RUNS.load(Ordering::Acquire) != 0 { break; }
        test_yield();
    }
    assert_eq!(REGISTERED_TARGET_RUNS.load(Ordering::Acquire), 1);
}

extern "C" fn kill_at_user_return(_arg: usize) -> ! {
    disable_interrupts();
    crate::sched::dispatch::run_user_kernel_entry(|| {
        current_thread().request_cancel();
    });

    panic!("a thread with a pending kill returned from its kernel entry");
}

fn test_thread_kill_retires_at_user_return() {
    let ptr = create_tcb(
        kill_at_user_return as *const () as usize,
        0,
        ThreadPriority::HIGH,
        KERNEL_PROCESS.clone(),
    ).expect("create user-return test thread");
    let pin = KERNEL_PROCESS.pinned_thread_snapshot().into_iter()
        .find(|pin| pin.as_ptr() == ptr)
        .expect("pin user-return test thread");

    crate::sched::dispatch::enqueue_ready_thread(ptr);
    for _ in 0..SMP_TEST_WAIT_ITERATIONS {
        if unsafe { (*pin.as_ptr()).state() } == ThreadState::Terminated {
            return;
        }
        test_yield();
    }

    panic!("thread kill did not retire the test thread");
}

pub fn run_concurrency_tests() {
    crate::klogln!(Info, "Test/Scheduler", "Thread kill retires at user return");
    test_thread_kill_retires_at_user_return();
    test_registered_wake_before_park();
    crate::klogln!(Info, "Test/Scheduler", "Termination preserves nested park");
    test_active_invocation_retains_inner_park();
    crate::klogln!(Info, "Test/Scheduler", "Early timer preserves quantum");
    test_early_timer_event_preserves_quantum();
    crate::klogln!(Info, "Test/Scheduler", "Priority validation");
    test_priority_from_clamps_invalid_values();
    crate::klogln!(Info, "Test/Scheduler", "Wake boost");
    test_woken_thread_receives_boost();
    crate::klogln!(Info, "Test/Scheduler", "Completed-quantum boost decay");
    test_boost_decays_once_per_completed_quantum();
    crate::klogln!(Info, "Test/Scheduler", "Atomic wake claim");
    test_two_cpu_wakes_enqueue_blocked_thread_once();
    crate::klogln!(Info, "Test/Scheduler", "Idle CPU steals migratable work");
    test_idle_cpu_obtains_migratable_work();
    crate::klogln!(Info, "Test/Scheduler", "Pinned thread is not donated");
    test_migration_disabled_thread_is_never_donated();
    crate::klogln!(Info, "Test/Scheduler", "Migrated wake routing");
    test_stolen_thread_wakeup_routes_to_assigned_cpu();
    crate::klogln!(Info, "Test/Scheduler", "Wake before block");
    test_wake_immediately_before_blocking_cancels_block();
    crate::klogln!(Info, "Test/Scheduler", "Wake after block");
    test_wake_immediately_after_blocking_makes_thread_ready();
}
