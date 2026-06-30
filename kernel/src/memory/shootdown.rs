use core::hint;
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use crate::cpu::{
    NUM_CORES,
    current_core_id,
    current_core_mut,
};

pub struct TlbShootdownInfo {
    pub addr: AtomicUsize,
    pub size: AtomicUsize,
    pub generation: AtomicUsize,
    pub counter: AtomicUsize,
}

pub struct ShootdownLock {
    locked: AtomicBool,
}

impl ShootdownLock {
    pub const fn new() -> Self {
        Self { locked: AtomicBool::new(false) }
    }

    pub fn lock(&self) -> ShootdownLockGuard<'_> {
        while self.locked.swap(true, Ordering::Acquire) {
            service_pending_shootdown();
            hint::spin_loop();
        }

        ShootdownLockGuard { lock: self }
    }

    fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

pub struct ShootdownLockGuard<'a> {
    lock: &'a ShootdownLock,
}

impl Drop for ShootdownLockGuard<'_> {
    fn drop(&mut self) {
        self.lock.unlock();
    }
}

pub static SHOOTDOWN_INFO: TlbShootdownInfo = TlbShootdownInfo {
    addr: AtomicUsize::new(0),
    size: AtomicUsize::new(0),
    generation: AtomicUsize::new(0),
    counter: AtomicUsize::new(0),
};

pub static SHOOTDOWN_LOCK: ShootdownLock = ShootdownLock::new();

pub fn service_pending_shootdown() {
    let generation = SHOOTDOWN_INFO.generation.load(Ordering::Acquire);
    if generation == 0 {
        return;
    }

    let core = current_core_mut();
    let seen = core.shootdown_generation.load(Ordering::Acquire);
    if seen == generation {
        return;
    }

    let addr = SHOOTDOWN_INFO.addr.load(Ordering::Acquire);
    let size = SHOOTDOWN_INFO.size.load(Ordering::Acquire);

    hal::mmu::flush_tlb_range(addr, size);

    core.shootdown_generation.store(generation, Ordering::Release);
    SHOOTDOWN_INFO.counter.fetch_sub(1, Ordering::AcqRel);
}

pub fn shootdown(addr: usize, size: usize) {
    let this_core_id = current_core_id();
    let _lock = SHOOTDOWN_LOCK.lock();

    SHOOTDOWN_INFO.addr.store(addr, Ordering::Release);
    SHOOTDOWN_INFO.size.store(size, Ordering::Release);
    SHOOTDOWN_INFO.counter.store(*NUM_CORES - 1, Ordering::Release);

    let generation = SHOOTDOWN_INFO.generation.fetch_add(1, Ordering::AcqRel) + 1;

    for id in 0..*NUM_CORES {
        if id == this_core_id {
            continue;
        }

        hal::ipi::send_tlb_shootdown_ipi(id);
    }

    hal::mmu::flush_tlb_range(addr, size);
    current_core_mut().shootdown_generation.store(generation, Ordering::Release);

    while SHOOTDOWN_INFO.counter.load(Ordering::Acquire) != 0 {
        service_pending_shootdown();
        hint::spin_loop();
    }
}
