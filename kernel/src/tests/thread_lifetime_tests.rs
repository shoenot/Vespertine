use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use core::sync::atomic::AtomicBool;
use alloc::sync::Arc;
use vespertine_abi::{ProcState, ProcTermReason};

use crate::object::handle::HandleTable;
use crate::process::{register_process, ProcTermination, Process, ProcessControlBlock};
use crate::security::credentials::Credentials;
use crate::cpu::{current_core_id, NUM_CORES};
use crate::sched::dispatch::{create_tcb, enqueue_ready_thread};
use crate::sched::priority::ThreadPriority;
use crate::time::{get_time, ns_to_ticks, sleep};
use crate::KERNEL_PROCESS;
use crate::sched::block::ThreadWakeRegistration;

const NO_TARGET: usize = usize::MAX;
const DEFERRED: u8 = 1;
const FREED: u8 = 2;

static WATCHED_TID: AtomicUsize = AtomicUsize::new(NO_TARGET);
static PHASE: AtomicU8 = AtomicU8::new(0);
static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn watch_thread(tid: usize) {
    FREE_COUNT.store(0, Ordering::Relaxed);
    PHASE.store(0, Ordering::Relaxed);
    WATCHED_TID.store(tid, Ordering::Release);
}

pub(crate) fn note_freed(tid: usize) {
    if WATCHED_TID.load(Ordering::Acquire) == tid {
        let previous = FREE_COUNT.fetch_add(1, Ordering::Relaxed);
        assert_eq!(previous, 0, "TCB {tid} was reaped twice");
        PHASE.store(FREED, Ordering::Release);
    }
}

pub(crate) fn note_deferred(tid: usize) {
    if WATCHED_TID.load(Ordering::Acquire) == tid {
        PHASE.store(DEFERRED, Ordering::Release);
    }
}

fn wait_for_phase(expected: u8) {
    let deadline = get_time().saturating_add(ns_to_ticks(5_000_000_000));

    loop {
        let phase = PHASE.load(Ordering::Acquire);
        if phase == expected {
            return;
        }
        assert!(phase < expected, "reaper freed the TCB while it was pinned");
        assert!(get_time() < deadline, "timed out waiting for phase {expected}");
        sleep(10_000_000);
    }
}

extern "C" fn exit_worker(expected_core: usize) -> ! {
    assert_eq!(current_core_id(), expected_core);
    crate::cpu::current_core_mut().scheduler.terminate(0);
}

pub(crate) fn run() {
    if *NUM_CORES < 2 {
        crate::klogln!(Info, "Test/Thread", "Thread lifetime tests skipped (require two CPUs)");
        return;
    }

    crate::klogln!(Info, "Test/Thread", "Pinned TCB defers reaping");

    let worker_core = if current_core_id() == 0 { 1 } else { 0 };
    let worker = create_tcb(
        exit_worker as *const () as usize,
        worker_core,
        ThreadPriority::HIGH,
        KERNEL_PROCESS.clone(),
    )
    .expect("failed to create pin test worker");

    let tid = unsafe {
        (*worker).pin_to_core(worker_core);
        (*worker).thread_id
    };

    let pin = KERNEL_PROCESS
        .pinned_thread_snapshot()
        .into_iter()
        .find(|pin| pin.as_ptr() == worker)
        .expect("worker missing from pinned snapshot");

    let target = unsafe { Arc::clone(&(*worker).wake_target) };
    let during_reap = ThreadWakeRegistration::new(Arc::clone(&target));
    let after_free = ThreadWakeRegistration::new(target);

    watch_thread(tid);
    enqueue_ready_thread(worker);
    wait_for_phase(DEFERRED);

    assert!(during_reap.is_active());
    assert!(
        !during_reap.wake(),
        "reaper deferred the TCB without detaching its wake target"
    );
    assert!(!during_reap.fired());

    drop(pin);
    wait_for_phase(FREED);

    assert!(after_free.is_active());
    assert!(
        !after_free.wake(),
        "an active registration accepted a wake after the TCB was freed"
    );
    assert!(!after_free.fired());

    WATCHED_TID.store(NO_TARGET, Ordering::Release);

    crate::klogln!(Info, "Test/Thread", "Terminate a suspended thread");
    test_terminate_suspended();
    crate::klogln!(Info, "Test/Thread", "Resume a suspended thread twice");
    test_resume_suspended_twice();
    crate::klogln!(Info, "Test/Thread", "Race resume against termination");
    test_resume_terminate_race();
    test_smp_address_space_reap();
    crate::klogln!(Info, "Test/Thread", "Passed");
}

static ENTRY_CALLS: AtomicUsize = AtomicUsize::new(0);
static MAY_EXIT: AtomicBool = AtomicBool::new(false);

extern "C" fn suspended_worker(_: usize) -> ! {
    ENTRY_CALLS.fetch_add(1, Ordering::Release);

    while !MAY_EXIT.load(Ordering::Acquire) {
        sleep(1_000_000);
    }

    crate::cpu::current_core_mut().scheduler.terminate(42)
}

fn make_suspended_child(name: &str, entry: extern "C" fn(usize) -> !) -> Process {
    ENTRY_CALLS.store(0, Ordering::Relaxed);
    MAY_EXIT.store(false, Ordering::Relaxed);

    let child = ProcessControlBlock::new_unregistered(
        HandleTable::new(),
        name.into(),
        Credentials::system(),
    ).expect("cannot create suspended test process");

    // use a tiny kernel entry point so we can count execution directly.
    let thread = create_tcb(
        entry as *const () as usize,
        0,
        ThreadPriority::HIGH,
        child.clone(),
    )
    .expect("failed to create suspended test thread");

    let tid = unsafe { (*thread).thread_id };
    watch_thread(tid);

    unsafe { child.install_initial_thread(thread) };
    register_process(&child);
    child
}

fn check_reaped(child: &Process, reason: ProcTermReason, code: u32, entries: usize) {
    wait_for_phase(FREED);
    assert!(child.vmm.read().is_none(), "reaped child still owns its VMM");

    let info = child.snapshot_info();
    assert_eq!(info.state, ProcState::Terminated);
    assert_eq!(info.active_threads, 0);
    assert_eq!(info.term_reason, reason);
    assert_eq!(info.term_code, code);
    assert_eq!(ENTRY_CALLS.load(Ordering::Acquire), entries);

    WATCHED_TID.store(NO_TARGET, Ordering::Release);
}

fn test_terminate_suspended() {
    let child = make_suspended_child("test-terminate-suspended", suspended_worker);

    assert!(child.request_terminate(ProcTermination::terminated(17)));
    check_reaped(&child, ProcTermReason::Terminated, 17, 0);
}

fn test_resume_suspended_twice() {
    let child = make_suspended_child("test-resume-suspended", suspended_worker);

    child.resume_initial_thread();
    child.resume_initial_thread();
    MAY_EXIT.store(true, Ordering::Release);

    check_reaped(&child, ProcTermReason::Exited, 42, 1);
}

struct StartupRace {
    child: Process,
    arrived: AtomicUsize,
    finished: AtomicUsize,
}

extern "C" fn race_child(_: usize) -> ! {
    ENTRY_CALLS.fetch_add(1, Ordering::Release);
    loop {
        core::hint::spin_loop();
    }
}

extern "C" fn race_controller(arg: usize) -> ! {
    // Each controller gets one Arc clone from the test setup.
    let race = unsafe { Arc::from_raw(arg as *const StartupRace) };
    let core = current_core_id();
    assert!(core < 2, "controller ran on an unexpected CPU");

    let bit = 1 << core;
    let previous = race.arrived.fetch_or(bit, Ordering::AcqRel);
    assert_eq!(previous & bit, 0, "both controllers ran on CPU {core}");

    let deadline = get_time().saturating_add(ns_to_ticks(5_000_000_000));
    while race.arrived.load(Ordering::Acquire) != 0b11 {
        assert!(get_time() < deadline, "other controller never reached gate");
        core::hint::spin_loop();
    }

    if core == 0 {
        race.child.resume_initial_thread();
    } else {
        assert!(race.child.request_terminate(ProcTermination::terminated(91)));
    }

    race.finished.fetch_add(1, Ordering::Release);
    drop(race);
    crate::cpu::current_core_mut().scheduler.terminate(0)
}

fn test_resume_terminate_race() {
    use hal::interrupts::{disable_interrupts, enable_interrupts, interrupts_enabled};

    let race = Arc::new(StartupRace {
        child: make_suspended_child("test-resume-terminate-race", race_child),
        arrived: AtomicUsize::new(0),
        finished: AtomicUsize::new(0),
    });

    let controllers: [*mut crate::sched::Thread; 2] = core::array::from_fn(|core| {
        let arg = Arc::into_raw(Arc::clone(&race)) as usize;
        let thread = create_tcb(
            race_controller as *const () as usize,
            arg,
            ThreadPriority::HIGH,
            KERNEL_PROCESS.clone(),
        )
        .expect("failed to create race controller");

        unsafe { (*thread).pin_to_core(core) };
        thread
    });

    // Publish both controllers before the local one can preempt this setup
    // and spin at the gate.
    let restore_interrupts = interrupts_enabled();
    disable_interrupts();
    for thread in controllers {
        enqueue_ready_thread(thread);
    }
    if restore_interrupts {
        enable_interrupts();
    }

    let deadline = get_time().saturating_add(ns_to_ticks(5_000_000_000));
    while race.finished.load(Ordering::Acquire) != 2 {
        assert!(get_time() < deadline, "Resume or Terminate did not return");
        sleep(10_000_000);
    }

    wait_for_phase(FREED);

    let info = race.child.snapshot_info();
    assert_eq!(info.state, ProcState::Terminated);
    assert_eq!(info.active_threads, 0);
    assert_eq!(info.term_reason, ProcTermReason::Terminated);
    assert_eq!(info.term_code, 91);
    assert_eq!(FREE_COUNT.load(Ordering::Acquire), 1);

    let entries = ENTRY_CALLS.load(Ordering::Acquire);
    assert!(entries <= 1, "child entry ran {entries} times");

    WATCHED_TID.store(NO_TARGET, Ordering::Release);
}

struct SmpReapWorker<'a> {
    core: usize,
    root: usize,
    addr: usize,
    marker: u8,
    bit: usize,
    entered: &'a AtomicUsize,
}

extern "C" fn smp_reap_worker(arg: usize) -> ! {
    // The controller keeps both argument records alive until both TCBs are reaped.
    let worker = unsafe { &*(arg as *const SmpReapWorker<'_>) };

    assert_eq!(current_core_id(), worker.core);
    let cr3 = hal::mmu::get_cr3() as usize & 0x000f_ffff_ffff_f000;
    assert_eq!(cr3, worker.root, "wrong address space on worker CPU");

    assert!(hal::usercopy::safe_copy_to(
        worker.addr as *mut u8,
        &worker.marker,
        1,
    ));
    worker.entered.fetch_or(worker.bit, Ordering::AcqRel);

    // Leave retirement to the scheduler after the controller requests exit.
    loop {
        core::hint::spin_loop();
    }
}

fn test_smp_address_space_reap() {
    use alloc::boxed::Box;
    use vespertine_abi::{AccessRights, Invocation, ProcInfo, ProcOp};

    use crate::executor::syscall_bridge::block_on;
    use crate::memory::PAGE_SIZE_4K as PAGE;
    use crate::memory::vmm::{VmOptions, VmaBacking};
    use crate::object::invoke::InvocationError;
    use crate::object::mempool::MemPool;
    use crate::object::vfs::{kernel_close, kernel_invoke, kernel_register_obj};
    use crate::sched::Thread;

    const REASON: u32 = 0x4d4d;

    let controller = current_core_id();
    let mut remote = (0..*NUM_CORES).filter(|&core| core != controller);
    let (Some(first), Some(second)) = (remote.next(), remote.next()) else {
        crate::klogln!(Info, "Test/Thread", "SMP address-space reap skipped (requires three CPUs)");
        return;
    };
    let cores = [first, second];

    const ROUNDS: usize = 4;
    crate::klogln!(Info, "Test/Thread", "SMP address-space reap ({ROUNDS} rounds)");
    for _ in 0..ROUNDS {
        let child = ProcessControlBlock::new_unregistered(
            HandleTable::new(),
            "smp-address-space-reap".into(),
            Credentials::system(),
        ).expect("create SMP test process");

        let handle = kernel_register_obj(
            child.clone(),
            AccessRights::READ | AccessRights::WRITE,
        );

        let pool = MemPool::new(Some(2 * PAGE), None);
        let backing = pool.allocate_backing(2 * PAGE).unwrap();
        let frames = [
            Arc::downgrade(&backing.get_page(0).unwrap()),
            Arc::downgrade(&backing.get_page(PAGE).unwrap()),
        ];

        let addr = {
            let mut slot = child.vmm.write();
            let vmm = slot.as_mut().unwrap();
            let addr = vmm.reserve(
                2 * PAGE,
                VmOptions::user_rw(),
                VmaBacking::Vmo(backing),
            ).unwrap();

            // Install both PTEs so the workers test access through the child's CR3.
            vmm.handle_page_fault(addr, 0b110).unwrap();
            vmm.handle_page_fault(addr + PAGE, 0b110).unwrap();
            addr
        };

        let entered = AtomicUsize::new(0);
        let args: [SmpReapWorker<'_>; 2] = core::array::from_fn(|i| SmpReapWorker {
            core: cores[i],
            root: child.root_addr,
            addr: addr + i * PAGE,
            marker: 0x40 + i as u8,
            bit: 1 << i,
            entered: &entered,
        });
        let workers: [*mut Thread; 2] = core::array::from_fn(|i| {
            let thread = create_tcb(
                smp_reap_worker as *const () as usize,
                (&args[i] as *const SmpReapWorker<'_>) as usize,
                ThreadPriority::HIGH,
                child.clone(),
            ).expect("create SMP test worker");

            unsafe { (*thread).pin_to_core(cores[i]) };
            thread
        });
        let tids = workers.map(|thread| unsafe { (*thread).thread_id });

        let pin = child.pinned_thread_snapshot().into_iter()
            .find(|pin| pin.as_ptr() == workers[0])
            .expect("first worker missing from child");

        register_process(&child);
        watch_thread(tids[1]);
        for thread in workers {
            enqueue_ready_thread(thread);
        }

        let deadline = get_time().saturating_add(ns_to_ticks(5_000_000_000));
        while entered.load(Ordering::Acquire) != 0b11 {
            assert!(get_time() < deadline, "both CPUs did not enter the child's VMM");
            sleep(10_000_000);
        }

        for (i, frame) in frames.iter().enumerate() {
            let frame = frame.upgrade().expect("worker's frame disappeared");
            let byte = unsafe { (frame.virt() as *const u8).read() };
            assert_eq!(byte, args[i].marker);
        }

        block_on(Box::pin(kernel_invoke(
            handle,
            Invocation::Proc(ProcOp::Terminate { reason: REASON }),
        ))).unwrap();

        wait_for_phase(FREED); // The unpinned worker is gone.
        watch_thread(tids[0]);
        wait_for_phase(DEFERRED); // The other worker has reached the reaper.

        assert!(child.is_terminated());
        assert!(child.vmm.read().is_some(), "VMM dropped before the final reap");
        assert!(matches!(
            pool.allocate_backing(1),
            Err(InvocationError::PoolExhausted),
        ));

        drop(pin);
        wait_for_phase(FREED);

        assert!(child.vmm.read().is_none(), "final reap left the VMM alive");
        assert!(frames.iter().all(|frame| frame.upgrade().is_none()));
        drop(pool.allocate_backing(2 * PAGE).expect("VMO quota was not refunded"));

        // Retain only the registered handle on the controller side.
        drop(child);
        let mut info = ProcInfo::zeroed();
        block_on(Box::pin(kernel_invoke(
            handle,
            Invocation::Proc(ProcOp::GetInfo {
                info_ptr: (&mut info as *mut ProcInfo) as usize,
            }),
        ))).unwrap();

        assert_eq!(info.state, ProcState::Terminated);
        assert_eq!(info.term_reason, ProcTermReason::Terminated);
        assert_eq!(info.term_code, REASON);
        assert_eq!(info.memory_usage, 0);

        kernel_close(handle).unwrap();
        WATCHED_TID.store(NO_TARGET, Ordering::Release);
    }
}
