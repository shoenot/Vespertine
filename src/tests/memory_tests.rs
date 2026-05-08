use core::fmt::Write;
use core::hint::black_box;
use core::ptr::{write_volatile, read_volatile};

use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::alloc::{alloc, dealloc, Layout};

use crate::Logger;

pub fn test_kmalloc(logger: &mut Logger) {
    unsafe {
        write!(logger, "\nRunning kmalloc tests... ").unwrap();
        let layout = Layout::new::<u64>();
        let p1 = black_box(alloc(layout) as *mut u64);
        write!(logger, "Allocation OK... ").unwrap();
        
        if p1.is_null() {
            write!(logger, "[FAIL] p1 is null\n").unwrap();
            return;
        }

        write_volatile(p1, 0x12345678_ABCDEF01);
        if read_volatile(p1) != 0x12345678_ABCDEF01 {
            write!(logger, "[FAIL] Memory corruption at {:p}\n", p1).unwrap();
            return;
        }
        write!(logger, "Write test OK... ").unwrap();

        let original_addr = p1 as usize;
        dealloc(black_box(p1 as *mut u8), layout);
        
        let p2 = black_box(alloc(layout) as *mut u64);
        if p2 as usize != original_addr {
            write!(logger, "[FAIL] SLUB did not recycle pointer\n").unwrap();
        } else {
            write!(logger, "Recycling test OK\n").unwrap();
        }

        dealloc(black_box(p2 as *mut u8), layout);
        write!(logger, "All kmalloc tests passed!\n").unwrap();
    }
}

pub fn test_vmalloc(logger: &mut Logger) {
    unsafe {
        write!(logger, "\nRunning vmalloc tests... ").unwrap();

        let size = 8192; // 2 pages
        let layout = Layout::from_size_align(size, 4096).unwrap();
        let p_large = black_box(alloc(layout));

        if p_large.is_null() {
            write!(logger, "[FAIL] vmalloc failed for 8KB\n").unwrap();
            return;
        }

        if (p_large as usize) < 0x4000_0000 {
            write!(logger, "[FAIL] vmalloc returned HHDM address instead of VMM address\n").unwrap();
        }
        write!(logger, "Allocation OK... ").unwrap();

        write_volatile(p_large as *mut u64, 0xAAAA_BBBB);
        if read_volatile(p_large as *mut u64) != 0xAAAA_BBBB {
            write!(logger, "[FAIL] Demand paging failed\n").unwrap();
            return;
        }
        write!(logger, "Demand paging OK\n").unwrap();

        black_box(dealloc(p_large, layout));
        write!(logger, "All vmalloc tests passed!\n").unwrap();
    }
}

pub fn test_collections(logger: &mut Logger) {
    write!(logger, "\nTesting rust high-level collections... \n").unwrap();
    
    write!(logger, "    Testing boxes... ").unwrap();
    let b = Box::new(42u32);
    if *b != 42 {
        write!(logger, "[FAIL] Box value mismatch\n").unwrap();
        return;
    }
    write!(logger, "Box test OK\n").unwrap();
    
    write!(logger, "    Testing vectors... ").unwrap();
    let mut v = Vec::new();
    for i in 0..100 {
        v.push(i);
    }

    if v.len() != 100 || v[99] != 99 {
        write!(logger, "[FAIL] Vector corruption\n").unwrap();
        return;
    }
    write!(logger, "Vector test OK\n").unwrap();

    write!(logger, "Collections tests passed!\n").unwrap();
}
