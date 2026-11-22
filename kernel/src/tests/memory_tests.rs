use crate::memory::vmo::AnonymousVmo;
use crate::memory::{ALLOCATOR, PAGE_SIZE_4K};
use crate::memory::vmm::*;
use crate::memory::{
    PageSize,
    GLOBAL_PMM,
    PAGE_SIZE_2M,
    range_tree::{
        RangeMap,
        RangeMapError,
    },
};

use crate::{klog, klogln};

pub(super) fn run() {
    run_pmm_tests();
    run_range_tree_tests();
    run_vmm_tests();
}

fn run_pmm_tests() {
    klogln!("RUNNING PMM BUDDY TESTS...");
    test_buddy_merge();
    test_huge_alignment();
    test_freelist_isolation();
    klogln!("ALL PMM TESTS PASSED!");
}

fn test_buddy_merge() {
    klog!("  Testing buddy split and merge... ");
    let mut pmm = GLOBAL_PMM.lock();

    // get order 1 block, and store the address, then free it
    let target_block = pmm.alloc_order(1).expect("Failed to alloc Order 1");
    pmm.free_order(target_block, 1);

    // get 2 order 0 blocks so they split the order 1 above
    let left_child = pmm.alloc_order(0).expect("Failed to alloc left Order 0");
    let right_child = pmm.alloc_order(0).expect("Failed to alloc right Order 0");

    // free the blocks, which should result in them merging back into the order 1 block
    pmm.free_order(right_child, 0);
    pmm.free_order(left_child, 0);

    // get the merged block addr
    let merged_block = pmm.alloc_order(1).expect("Failed to re-alloc Order 1");

    assert_eq!(target_block, merged_block, "Buddy merge failed! Expected base {:#X}, got {:#X}", target_block, merged_block);

    pmm.free_order(merged_block, 1);
    klogln!("OK");
}

fn test_huge_alignment() {
    klog!("  Testing huge page alignment... ");
    let mut pmm = GLOBAL_PMM.lock();

    let huge_frame = pmm.alloc(PageSize::Size2M).expect("Failed to allocate Huge Page");

    assert_eq!(huge_frame % PAGE_SIZE_2M, 0, "Alignment fault! {:#X} is not 2MB aligned.", huge_frame);

    pmm.free(huge_frame, PageSize::Size2M);
    klogln!("OK");
}

fn test_freelist_isolation() {
    klog!("  Testing freelist isolation... ");
    let mut pmm = GLOBAL_PMM.lock();

    // Allocate two blocks, ensure the allocator doesn't hand out the same frame twice.
    let block1 = pmm.alloc(PageSize::Size4K).unwrap();
    let block2 = pmm.alloc(PageSize::Size4K).unwrap();

    assert!(block1 != block2, "Allocator handed out the same frame twice! {:#X}", block1);

    pmm.free(block1, PageSize::Size4K);
    pmm.free(block2, PageSize::Size4K);
    klogln!("OK");
}

fn run_range_tree_tests() {
    klogln!("RUNNING RANGE TREE TESTS...");
    test_range_tree_insert_lookup();
    test_range_tree_overlap_rejection();
    test_range_tree_gap_search();
    test_range_tree_remove_stress();
    klogln!("ALL RANGE TREE TESTS PASSED!");
}

fn test_range_tree_insert_lookup() {
    klog!("  Testing range tree insert/lookup... ");
    let mut tree = RangeMap::new();

    assert_eq!(tree.insert(0x3000, 0x4000, 3), Ok(()));
    assert_eq!(tree.insert(0x1000, 0x2000, 1), Ok(()));
    assert_eq!(tree.insert(0x5000, 0x6000, 5), Ok(()));

    assert_eq!(*tree.get(0x1000).expect("missing first range").value, 1);
    assert_eq!(*tree.get(0x3FFF).expect("missing middle range").value, 3);
    assert_eq!(*tree.get(0x5001).expect("missing last range").value, 5);

    assert!(tree.get(0x2000).is_none());
    assert!(tree.get(0x4000).is_none());
    assert!(tree.validate());

    klogln!("OK");
}

fn test_range_tree_overlap_rejection() {
    klog!("  Testing range tree overlap rejection... ");
    let mut tree = RangeMap::new();

    assert_eq!(tree.insert(0x1000, 0x2000, 1), Ok(()));
    assert_eq!(tree.insert(0x2000, 0x3000, 2), Ok(()));

    assert_eq!(tree.insert(0x0800, 0x1001, 3), Err(RangeMapError::Overlap));
    assert_eq!(tree.insert(0x1800, 0x2800, 4), Err(RangeMapError::Overlap));
    assert_eq!(tree.insert(0x2FFF, 0x4000, 5), Err(RangeMapError::Overlap));
    assert_eq!(tree.insert(0x4000, 0x4000, 6), Err(RangeMapError::Empty));

    assert!(tree.validate());

    klogln!("OK");
}

fn test_range_tree_gap_search() {
    klog!("  Testing range tree gap search... ");
    let mut tree = RangeMap::new();

    assert_eq!(tree.insert(0x1000, 0x2000, 1), Ok(()));
    assert_eq!(tree.insert(0x4000, 0x5000, 2), Ok(()));
    assert_eq!(tree.insert(0x8000, 0x9000, 3), Ok(()));

    assert_eq!(tree.find_gap(0x1000, 0x1000, 0x1000, 0xA000), Ok(Some(0x2000)));
    assert_eq!(tree.find_gap(0x2000, 0x1000, 0x1000, 0xA000), Ok(Some(0x2000)));
    assert_eq!(tree.find_gap(0x3000, 0x1000, 0x1000, 0xA000), Ok(Some(0x5000)));
    assert_eq!(tree.find_gap(0x2000, 0x2000, 0x1000, 0xA000), Ok(Some(0x2000)));
    assert_eq!(tree.find_gap(0x2000, 0x2000, 0x5000, 0x8000), Ok(Some(0x6000)));
    assert_eq!(tree.find_gap(0x3000, 0x2000, 0x1000, 0xA000), Ok(None));
    assert_eq!(tree.find_gap(0x2000, 0x1000, 0x9000, 0xA000), Ok(None));

    assert!(tree.validate());

    klogln!("OK");
}

fn test_range_tree_remove_stress() {
    klog!("  Testing range tree repeated remove/rebalance... ");
    let mut tree = RangeMap::new();

    for i in 0..128 {
        let start = 0x1000 + i * 0x3000;
        assert_eq!(tree.insert(start, start + 0x1000, i), Ok(()));
        assert!(tree.validate());
    }

    for i in (0..128).step_by(2) {
        let start = 0x1000 + i * 0x3000;
        assert_eq!(tree.remove(start), Some(i));
        assert!(tree.validate());
    }

    for i in (1..128).step_by(2) {
        let start = 0x1000 + i * 0x3000;
        assert_eq!(*tree.get(start).expect("odd range disappeared").value, i);
    }

    for i in (1..128).rev().step_by(2) {
        let start = 0x1000 + i * 0x3000;
        assert_eq!(tree.remove(start), Some(i));
        assert!(tree.validate());
    }

    assert!(tree.is_empty());
    assert!(tree.validate());

    klogln!("OK");
}

fn run_vmm_tests() {
    klogln!("RUNNING VMM TESTS...");
    test_vmm_unmap_across_multiple_vmas();
    test_vmm_protect_middle_splits_vma();
    test_vmm_range_edits_reject_holes();
    test_vmm_transaction_stress();
    test_owned_anonymous_mappings();

    klogln!("ALL VMM TESTS PASSED!");
}

fn test_vmm_unmap_across_multiple_vmas() {
    klog!("  Testing VMM multi-vma unmap... ");
    let mut vmm = VirtMemManager::new(&ALLOCATOR);
    let base = 0x4000_0000;
    let page = PAGE_SIZE_4K;

    assert_eq!(
        vmm.map_at(base, page * 2, VmOptions::user_rw(), VmaBacking::Vmo(AnonymousVmo::new(page * 2)), 0, MapBehavior::RequireVacant),
        Ok(base),
    );
    assert_eq!(
        vmm.map_at(base + page * 2, page * 2, VmOptions::user_ro(), VmaBacking::Vmo(AnonymousVmo::new(page * 2)), 0, MapBehavior::RequireVacant),
        Ok(base + page * 2),
    );

    assert_eq!(vmm.unmap_range(base + page, page * 2), Ok(()));

    let left = vmm.find_vma(base).expect("left remainder missing");
    assert_eq!(left.start, base);
    assert_eq!(left.end, base + page);
    assert_eq!(left.value.permissions, VmOptions::user_rw().permissions);

    assert_eq!(vmm.find_vma(base + page), Err(VmError::NotFound));
    assert_eq!(vmm.find_vma(base + page * 2), Err(VmError::NotFound));

    let right = vmm.find_vma(base + page * 3).expect("right remainder missing");
    assert_eq!(right.start, base + page * 3);
    assert_eq!(right.end, base + page * 4);
    assert_eq!(right.value.permissions, VmOptions::user_ro().permissions);
    assert_eq!(right.value.backing_offset, page);

    assert!(vmm.validate());

    let snapshot = vmm.accounting().snapshot();
    assert_eq!(snapshot.reserved_bytes, page * 2);
    assert_eq!(snapshot.committed_bytes, page * 2);

    klogln!("OK");
}

fn test_vmm_protect_middle_splits_vma() {
    klog!("  Testing VMM middle protect split... ");
    let mut vmm = VirtMemManager::new(&ALLOCATOR);
    let base = 0x4000_0000;
    let page = PAGE_SIZE_4K;
    let size = page * 4;

    assert_eq!(
        vmm.map_at(base, size, VmOptions::user_rw(), VmaBacking::Vmo(AnonymousVmo::new(size)), 0, MapBehavior::RequireVacant),
        Ok(base),
    );

    let new_permissions = VmOptions::user_ro().permissions;
    assert_eq!(vmm.protect_range(base + page, page * 2, new_permissions), Ok(()));

    let left = vmm.find_vma(base).expect("left fragment missing");
    assert_eq!(left.start, base);
    assert_eq!(left.end, base + page);
    assert_eq!(left.value.permissions, VmOptions::user_rw().permissions);
    assert_eq!(left.value.backing_offset, 0);

    let middle = vmm.find_vma(base + page).expect("middle fragment missing");
    assert_eq!(middle.start, base + page);
    assert_eq!(middle.end, base + page * 3);
    assert_eq!(middle.value.permissions, new_permissions);
    assert_eq!(middle.value.backing_offset, page);

    let right = vmm.find_vma(base + page * 3).expect("right fragment missing");
    assert_eq!(right.start, base + page * 3);
    assert_eq!(right.end, base + page * 4);
    assert_eq!(right.value.permissions, VmOptions::user_rw().permissions);
    assert_eq!(right.value.backing_offset, page * 3);

    assert!(vmm.validate());

    let snapshot = vmm.accounting().snapshot();
    assert_eq!(snapshot.reserved_bytes, size);
    assert_eq!(snapshot.committed_bytes, size);

    klogln!("OK");
}

fn test_vmm_range_edits_reject_holes() {
    let mut vmm = VirtMemManager::new(&ALLOCATOR);
    let base = 0x4000_0000;
    let page = PAGE_SIZE_4K;
    let left = VmOptions::user_rw();
    let right = VmOptions::user_rx();

    for (addr, options) in [(base, left), (base + 2 * page, right)] {
        vmm.map_at(
            addr,
            page,
            options,
            VmaBacking::Vmo(AnonymousVmo::new(page)),
            0,
            MapBehavior::RequireVacant,
        ).unwrap();
    }

    let before = vmm.accounting().snapshot();
    let check_unchanged = |vmm: &VirtMemManager| {
        for (addr, options) in [(base, left), (base + 2 * page, right)] {
            let entry = vmm.find_vma(addr).expect("range edit removed a mapping");
            assert_eq!((entry.start, entry.end), (addr, addr + page));
            assert_eq!(entry.value.permissions, options.permissions);
        }
        assert_eq!(vmm.find_vma(base + page), Err(VmError::NotFound));
        let after = vmm.accounting().snapshot();
        assert_eq!(after.reserved_bytes, before.reserved_bytes);
        assert_eq!(after.committed_bytes, before.committed_bytes);
        assert!(vmm.validate());
    };

    assert_eq!(vmm.unmap_range(base, 3 * page), Err(VmError::NotFound));
    check_unchanged(&vmm);

    let read_only = VmOptions::user_ro().permissions;
    assert_eq!(vmm.protect_range(base, 3 * page, read_only), Err(VmError::NotFound));
    check_unchanged(&vmm);
    klogln!("  Unmap and protect reject holes without changing mappings");
}

fn test_vmm_transaction_stress() {
    klog!("  Testing VMM transaction stress pipeline... ");

    let mut vmm = VirtMemManager::new(&ALLOCATOR);
    let page = PAGE_SIZE_4K;
    let huge = PAGE_SIZE_2M;
    let size_10m = 10 * huge;

    let before = vmm.accounting().snapshot();

    // reserve 10mb
    let base = vmm.reserve(size_10m, VmOptions::user_rw(), VmaBacking::Reserved)
        .expect("Failed to reserve 10MB");

    // map 2mb exactly in the center
    assert_eq!(
        vmm.map_at(base + huge * 2, huge, VmOptions::user_rw(), VmaBacking::Vmo(AnonymousVmo::new(huge)), 0, MapBehavior::ReplaceContained),
        Ok(base + huge * 2)
    );

    // protect exactly 4k inside that 2mb chunk to ro
    assert_eq!(
        vmm.protect_range(base + huge * 2 + page, page, VmOptions::user_ro().permissions),
        Ok(())
    );

    // protect the entire 10mb block back to read-write
    assert_eq!(
        vmm.protect_range(base, size_10m, VmOptions::user_rw().permissions),
        Ok(())
    );

    // unmap everything
    assert_eq!(
        vmm.unmap_range(base, size_10m),
        Ok(())
    );

    assert!(vmm.validate());

    let after = vmm.accounting().snapshot();
    assert_eq!(before.reserved_bytes, after.reserved_bytes);
    assert_eq!(before.committed_bytes, after.committed_bytes);

    klogln!("OK");
}

fn test_owned_anonymous_mappings() {
    use alloc::sync::Arc;
    use crate::memory::vmo::AnonymousVmo;

    const USER_NOT_PRESENT: usize = 0x4;

    let page = PAGE_SIZE_4K;
    let left = 0x4000_0000;
    let right = left + page;

    let mut vmm = VirtMemManager::new(&ALLOCATOR);
    let vmo = AnonymousVmo::new(page);
    let frame = vmo.get_page(0).expect("allocate test page");
    let weak = Arc::downgrade(&frame);

    for va in [left, right] {
        assert_eq!(
            vmm.map_at(
                va,
                page,
                VmOptions::user_rw(),
                VmaBacking::Vmo(vmo.clone()),
                0,
                MapBehavior::RequireVacant,
            ),
            Ok(va),
        );
    }

    let check = |vmm: &VirtMemManager, refs, resident| {
        assert_eq!(Arc::strong_count(&frame), refs, "frame reference count");
        assert_eq!(
            vmm.accounting().snapshot().resident_bytes,
            resident,
            "resident mapping bytes",
        );
    };

    check(&vmm, 2, 0);
    assert_eq!(vmm.handle_page_fault(left, USER_NOT_PRESENT), Ok(()));
    check(&vmm, 3, page);
    assert_eq!(vmm.handle_page_fault(right, USER_NOT_PRESENT), Ok(()));
    check(&vmm, 4, 2 * page);

    assert_eq!(vmm.handle_page_fault(left, USER_NOT_PRESENT), Ok(()));
    check(&vmm, 4, 2 * page);

    assert_eq!(vmm.unmap_range(left, page), Ok(()));
    check(&vmm, 3, page);
    assert_eq!(vmm.handle_page_fault(right, USER_NOT_PRESENT), Ok(()));
    check(&vmm, 3, page);

    assert_eq!(vmm.unmap_range(right, page), Ok(()));
    check(&vmm, 2, 0);

    drop(frame);
    drop(vmo);
    assert!(weak.upgrade().is_none(), "an owner leaked the frame");

    klogln!("  Owned anonymous mappings OK");
}
