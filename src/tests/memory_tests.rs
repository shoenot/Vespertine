use alloc::alloc::{
    Layout,
    alloc,
    dealloc,
};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hint::black_box;
use core::ptr::{
    read_volatile,
    write_volatile,
};

use crate::{
    klog,
    klogln,
    vklog,
    vklogln,
};
use crate::memory::{PCAllocator, GLOBAL_PMM, BlockSize};
use crate::memory::HUGE_PAGE_SIZE;


pub fn test_kmalloc(print: bool) {
    unsafe {
        vklogln!(print, "");
        vklog!(print, "Running kmalloc tests... ");
        let layout = Layout::new::<u64>();
        let p1 = black_box(alloc(layout) as *mut u64);
        vklog!(print, "Allocation OK... ");

        if p1.is_null() {
            vklogln!(print, "[FAIL] p1 is null");
            panic!("MEMORY TEST FAILED");
        }

        write_volatile(p1, 0x12345678_ABCDEF01);
        if read_volatile(p1) != 0x12345678_ABCDEF01 {
            vklogln!(print, "[FAIL] Memory corruption at {:p}", p1);
            panic!("MEMORY TEST FAILED");
        }
        vklog!(print, "Write test OK... ");

        let original_addr = p1 as usize;
        dealloc(black_box(p1 as *mut u8), layout);

        let p2 = black_box(alloc(layout) as *mut u64);
        if p2 as usize != original_addr {
            vklogln!(print, "[FAIL] SLUB did not recycle pointer");
            panic!("MEMORY TEST FAILED");
        } else {
            vklogln!(print, "Recycling test OK");
        }

        dealloc(black_box(p2 as *mut u8), layout);
        vklogln!(print, "All kmalloc tests passed!");
    }
}

pub fn test_vmalloc(print: bool) {
    unsafe {
        vklogln!(print, "");
        vklog!(print, "Running vmalloc tests... ");

        let size = 8192; // 2 pages
        let layout = Layout::from_size_align(size, 4096).unwrap();
        let p_large = black_box(alloc(layout));

        if p_large.is_null() {
            vklogln!(print, "[FAIL] vmalloc failed for 8KB");
            panic!("MEMORY TEST FAILED");
        }

        if (p_large as usize) < 0x4000_0000 {
            vklog!(print, "[FAIL] vmalloc returned HHDM address instead of VMM address\n");
            panic!("MEMORY TEST FAILED");
        }
        vklog!(print, "Allocation OK... ");

        write_volatile(p_large as *mut u64, 0xAAAA_BBBB);
        if read_volatile(p_large as *mut u64) != 0xAAAA_BBBB {
            vklog!(print, "[FAIL] Demand paging failed");
            panic!("MEMORY TEST FAILED");
        }
        vklogln!(print, "Demand paging OK");

        black_box(dealloc(p_large, layout));
        vklogln!(print, "All vmalloc tests passed!");
    }
}

pub fn test_collections(print: bool) {
    vklogln!(print, "");
    vklogln!(print, "Testing rust high-level collections... ");

    vklog!(print, "    Testing boxes... ");
    let b = Box::new(42u32);
    if *b != 42 {
        vklogln!(print, "[FAIL] Box value mismatch");
        panic!("MEMORY TEST FAILED");
    }
    vklogln!(print, "Box test OK");

    vklog!(print, "    Testing vectors... ");
    let mut v = Vec::new();
    for i in 0..100 {
        v.push(i);
    }

    if v.len() != 100 || v[99] != 99 {
        vklogln!(print, "[FAIL] Vector corruption");
        panic!("MEMORY TEST FAILED");
    }
    vklogln!(print, "Vector test OK");

    vklogln!(print, "Collections tests passed!");
}

pub fn run_pmm_tests() {
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
    
    assert_eq!(
        target_block, merged_block, 
        "Buddy merge failed! Expected base {:#X}, got {:#X}", 
        target_block, merged_block
    );

    pmm.free_order(merged_block, 1);
    klogln!("OK");
}

fn test_huge_alignment() {
    klog!("  Testing huge page alignment... ");
    let mut pmm = GLOBAL_PMM.lock();

    let huge_frame = pmm.alloc(BlockSize::Huge).expect("Failed to allocate Huge Page");
    
    assert_eq!(
        huge_frame % HUGE_PAGE_SIZE, 
        0, 
        "Alignment fault! {:#X} is not 2MB aligned.", 
        huge_frame
    );

    pmm.free(huge_frame, BlockSize::Huge);
    klogln!("OK");
}

fn test_freelist_isolation() {
    klog!("  Testing freelist isolation... ");
    let mut pmm = GLOBAL_PMM.lock();

    // Allocate two blocks, ensure the allocator doesn't hand out the same frame twice.
    let block1 = pmm.alloc(BlockSize::Normal).unwrap();
    let block2 = pmm.alloc(BlockSize::Normal).unwrap();

    assert!(block1 != block2, "Allocator handed out the same frame twice! {:#X}", block1);

    pmm.free(block1, BlockSize::Normal);
    pmm.free(block2, BlockSize::Normal);
    klogln!("OK");
}
