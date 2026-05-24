use core::arch::asm;

#[inline(always)]
pub unsafe fn write_to_msr(val: u64, msr: u32) { 
    unsafe {
        asm!("wrmsr",
             in("ecx") msr,
             in("edx") (val >> 32) as u32,
             in("eax") val as u32,
             options(nomem, nostack, preserves_flags),
        );
    }
}

#[inline(always)]
pub unsafe fn read_from_msr(msr: u32) -> u64 {
    let (mut hi, mut lo): (u32, u32);
    unsafe {
        asm!("rdmsr",
             in("ecx") msr,
             lateout("edx") hi,
             lateout("eax") lo,
        );
    }
    ((hi as u64) << 32) | lo as u64
}
