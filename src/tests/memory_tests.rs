use core::fmt::Write;
use core::hint::black_box;
use core::ptr::{write_volatile, read_volatile};

use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::alloc::{alloc, dealloc, Layout};

use crate::{klogln, klog};

pub fn test_kmalloc() {
    unsafe {
        klogln!("");
        klog!("Running kmalloc tests... ");
        let layout = Layout::new::<u64>();
        let p1 = black_box(alloc(layout) as *mut u64);
        klog!("Allocation OK... ");
        
        if p1.is_null() {
            klogln!("[FAIL] p1 is null");
            panic!("MEMORY TEST FAILED");
        }

        write_volatile(p1, 0x12345678_ABCDEF01);
        if read_volatile(p1) != 0x12345678_ABCDEF01 {
            klogln!("[FAIL] Memory corruption at {:p}", p1);
            panic!("MEMORY TEST FAILED");
        }
        klog!("Write test OK... ");

        let original_addr = p1 as usize;
        dealloc(black_box(p1 as *mut u8), layout);
        
        let p2 = black_box(alloc(layout) as *mut u64);
        if p2 as usize != original_addr {
            klogln!("[FAIL] SLUB did not recycle pointer");
            panic!("MEMORY TEST FAILED");
        } else {
            klogln!("Recycling test OK");
        }

        dealloc(black_box(p2 as *mut u8), layout);
        klogln!("All kmalloc tests passed!");
    }
}

pub fn test_vmalloc() {
    unsafe {
        klogln!("");
        klog!("Running vmalloc tests... ");

        let size = 8192; // 2 pages
        let layout = Layout::from_size_align(size, 4096).unwrap();
        let p_large = black_box(alloc(layout));

        if p_large.is_null() {
            klogln!("[FAIL] vmalloc failed for 8KB");
            panic!("MEMORY TEST FAILED");
        }

        if (p_large as usize) < 0x4000_0000 {
            klog!("[FAIL] vmalloc returned HHDM address instead of VMM address\n");
            panic!("MEMORY TEST FAILED");
        }
        klog!("Allocation OK... ");

        write_volatile(p_large as *mut u64, 0xAAAA_BBBB);
        if read_volatile(p_large as *mut u64) != 0xAAAA_BBBB {
            klog!("[FAIL] Demand paging failed");
            panic!("MEMORY TEST FAILED");
        }
        klogln!("Demand paging OK");

        black_box(dealloc(p_large, layout));
        klogln!("All vmalloc tests passed!");
    }
}

pub fn test_collections() {
    klogln!("");
    klogln!("Testing rust high-level collections... ");
    
    klog!("    Testing boxes... ");
    let b = Box::new(42u32);
    if *b != 42 {
        klogln!("[FAIL] Box value mismatch");
        panic!("MEMORY TEST FAILED");
    }
    klogln!("Box test OK");
    
    klog!("    Testing vectors... ");
    let mut v = Vec::new();
    for i in 0..100 {
        v.push(i);
    }

    if v.len() != 100 || v[99] != 99 {
        klogln!("[FAIL] Vector corruption");
        panic!("MEMORY TEST FAILED");
    }
    klogln!("Vector test OK");

    klogln!("Collections tests passed!");
}
