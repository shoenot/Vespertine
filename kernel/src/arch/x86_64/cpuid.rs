use core::arch::x86_64::{
    __cpuid,
    __cpuid_count,
};

use crate::util::bitwise::check_bit;

// TIMER

pub fn has_invariant_tsc() -> bool {
    let highest = __cpuid(0x80000000u32);
    if highest.eax < 0x80000007u32 {
        false
    } else {
        let ret = __cpuid(0x80000007u32);
        (ret.edx & (1 << 8)) != 0
    }
}

pub fn has_tsc_deadline() -> bool {
    let leaf1 = __cpuid(0x01);
    (leaf1.ecx & (1 << 24)) != 0
}

pub fn check_tsc_frequency() -> Option<usize> {
    let fq = __cpuid(0x15);
    if (fq.eax == 0) || (fq.ebx == 0) || (fq.ecx == 0) {
        None
    } else {
        let tsc_fq = (fq.ecx as usize * fq.ebx as usize) / fq.eax as usize;
        Some(tsc_fq)
    }
}

pub fn check_apic_frequency() -> Option<usize> {
    let fq = __cpuid(0x15);
    if fq.ecx == 0 {
        None
    } else {
        let apic_fq = fq.ecx as usize / 16;
        Some(apic_fq)
    }
}

// FPU/AVX

/// EAX: Valid bits for XCR0
/// EBX: Required buffer size based on XCR0 enabled bits
/// ECX: Max possible size if everything enabled
pub fn get_xsave_details() -> (usize, usize, usize) {
    let size = __cpuid_count(0xD, 0);
    (size.eax as usize, size.ebx as usize, size.ecx as usize)
}

pub fn check_xsave_support() -> bool {
    let leaf1 = __cpuid(1);
    check_bit(leaf1.ecx, 26)
}
