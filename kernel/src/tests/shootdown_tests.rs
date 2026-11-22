use core::hint::spin_loop;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use hal::interrupts::{disable_interrupts, enable_interrupts, interrupts_enabled};
use hal::mmu::pager::Pager;
use hal::mmu::{get_cr3, load_cr3, PageFlags, PhysAddr, VirtAddr};

use crate::cpu::{current_core, current_core_id, NUM_CORES};
use crate::memory::shootdown::shootdown;
use crate::memory::{Frame4K, PageSize, PCAllocator, PAGER};
use crate::time::{get_time, ns_to_ticks};

const TEST_VA: usize = 0x4000_0000;
const FIRST_VALUE: u64 = 0x1111_1111_1111_1111;
const SECOND_VALUE: u64 = 0x2222_2222_2222_2222;

static TEST_ROOT: AtomicUsize = AtomicUsize::new(0);
static AP_PRIMED: AtomicBool = AtomicBool::new(false);
static SHOOTDOWN_DONE: AtomicBool = AtomicBool::new(false);
static AP_FINISHED: AtomicBool = AtomicBool::new(false);

fn wait_for(what: &str, ready: impl Fn() -> bool) {
    let deadline = get_time().saturating_add(ns_to_ticks(5_000_000_000));
    while !ready() {
        assert!(get_time() < deadline, "shootdown test timed out: {what}");
        spin_loop();
    }
}

pub(crate) fn run_bsp() {
    if *NUM_CORES < 2 {
        crate::klogln!("[TEST] cross-core TLB shootdown skipped, requires two CPUs");
        return;
    }

    crate::klogln!("[TEST] cross-core TLB shootdown");

    let root = Frame4K::allocate_zeroed().expect("test root allocation failed");
    let first = Frame4K::allocate().expect("first test frame allocation failed");
    let second = Frame4K::allocate().expect("second test frame allocation failed");

    unsafe {
        write_volatile(first.virt() as *mut u64, FIRST_VALUE);
        write_volatile(second.virt() as *mut u64, SECOND_VALUE);
    }

    let mut pager = Pager::new(PhysAddr(root.phys()));
    pager.sync_kernel_mappings(PAGER.lock().root_phys());

    let mut alloc = PCAllocator {};
    let virt = VirtAddr(TEST_VA);
    let size = PageSize::Size4K;
    let flags = PageFlags::NO_EXECUTE;

    pager
        .map_page(virt, PhysAddr(first.phys()), flags, size, &mut alloc)
        .expect("initial test mapping failed");

    TEST_ROOT.store(root.phys(), Ordering::Release);
    wait_for("AP read of frame A", || AP_PRIMED.load(Ordering::Acquire));

    pager.unmap_page_no_flush(virt, size).expect("test unmap failed");
    pager
        .map_page(virt, PhysAddr(second.phys()), flags, size, &mut alloc)
        .expect("replacement test mapping failed");

    shootdown(TEST_VA, size.bytes());
    SHOOTDOWN_DONE.store(true, Ordering::Release);

    wait_for("AP restored its original CR3", || {
        AP_FINISHED.load(Ordering::Acquire)
    });

    pager.free_user_page_tables(&mut alloc);
    crate::klogln!("[TEST] cross-core TLB shootdown OK");
}

pub(crate) fn run_ap() {
    if current_core_id() != 1 {
        return;
    }

    wait_for("test page-table root", || {
        TEST_ROOT.load(Ordering::Acquire) != 0
    });

    let test_cr3 = TEST_ROOT.load(Ordering::Acquire) as u64;
    let saved_cr3 = get_cr3();
    let restore_interrupts = interrupts_enabled();

    disable_interrupts();
    load_cr3(test_cr3);

    let before = current_core().shootdown_generation.load(Ordering::Acquire);
    let ptr = TEST_VA as *const u64;
    assert_eq!(unsafe { read_volatile(ptr) }, FIRST_VALUE);

    // The AP must be able to receive the shootdown IPI while it waits.
    enable_interrupts();
    AP_PRIMED.store(true, Ordering::Release);
    wait_for("BSP shootdown completion", || {
        SHOOTDOWN_DONE.load(Ordering::Acquire)
    });

    assert_eq!(get_cr3(), test_cr3, "AP switched address spaces");
    let after = current_core().shootdown_generation.load(Ordering::Acquire);
    let actual = unsafe { read_volatile(ptr) };

    disable_interrupts();
    load_cr3(saved_cr3);
    if restore_interrupts {
        enable_interrupts();
    }

    assert_ne!(after, before, "AP never acknowledged the shootdown");
    assert_eq!(actual, SECOND_VALUE, "AP still sees frame A");

    // Only now may the BSP release the scratch page tables and frames.
    AP_FINISHED.store(true, Ordering::Release);
}
