use core::arch::asm;

#[inline(always)]
pub fn get_cr3() -> u64 {
    let cr3: u64;
    unsafe {
        asm!("mov {0}, cr3", 
            out(reg) cr3,
            options(nostack, preserves_flags));
    };
    cr3
}

#[inline(always)]
pub fn load_cr3(addr: u64) {
    unsafe {
        asm!("mov cr3, {0}",
            in(reg) addr,
            options(nostack, preserves_flags));
    };
}

#[inline(always)]
pub fn flush_tlb(virt: u64) {
    unsafe {
        asm!("invlpg [{0}]", 
            in(reg) virt,
            options(nostack, preserves_flags))
    }
}

#[inline(always)]
pub fn flush_tlb_range(start: usize, size: usize) {
    let end = start.saturating_add(size);
    let mut current = start & !0xFFF;

    while current < end {
        flush_tlb(current as u64);
        current = current.saturating_add(4096);
    }
}

