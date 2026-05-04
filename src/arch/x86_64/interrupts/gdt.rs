use core::arch::asm;
use core::ptr::addr_of;
use core::mem::size_of_val;
use lazy_static::lazy_static;
use crate::drivers::serial::log_to_serial;

#[repr(C, packed)]
struct GDTEntry {
    limit_low: u16,
    base_low: u16,
    base_middle: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

#[repr(C, packed)]
struct GDTPointer {
    limit: u16,
    base: u64,
}

impl GDTEntry {
    const fn new(access: u8, flags: u8) -> Self {
        GDTEntry {
            limit_low: 0xFFFF,
            base_low: 0,
            base_middle: 0,
            access,
            granularity: flags | 0x0F,
            base_high: 0,
        }
    }
}

#[repr(C, packed)]
struct TaskStateSegment {
    reserved_1: u32,
    rsp: [u64; 3],
    reserved_2: u64,
    ist: [u64; 7],
    reserved_3: u64,
    reserved_4: u16,
    iomap_base: u16,
}

#[repr(C, packed)]
struct TSSDescriptor {
    low: GDTEntry,
    high_base: u32,
    _reserved: u32,
}

lazy_static!(
    static ref TSS_INSTANCE: TaskStateSegment = {
        let mut tss = TaskStateSegment {
            reserved_1: 0,
            rsp: [0; 3],
            reserved_2: 0,
            ist: [0; 7],
            reserved_3: 0,
            reserved_4: 0,
            iomap_base: 104,
        };
        tss
    };

    static ref GDT: [GDTEntry; 7] = {
        let mut gdt = [
            GDTEntry { limit_low: 0, base_low: 0, base_middle: 0, access: 0, granularity: 0, base_high: 0 },
            GDTEntry::new(0x9A, 0xA0),
            GDTEntry::new(0x92, 0xA0),
            GDTEntry::new(0xFA, 0xA0),
            GDTEntry::new(0xF2, 0xA0),
            GDTEntry::new(0, 0), // for tss
            GDTEntry::new(0, 0), // for tss
        ];

        let tss_base = &*TSS_INSTANCE as *const TaskStateSegment as u64;
        let tss_limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u16;
        let tss_base_high = (tss_base >> 32) as u32;

        gdt[5] = GDTEntry {
            limit_low: tss_limit,
            base_low: tss_base as u16,
            base_middle: (tss_base >> 16) as u8,
            access: 0x89,
            granularity: 0,
            base_high: (tss_base >> 24) as u8,
        };
        
        gdt[6] = GDTEntry { 
            limit_low: tss_base_high as u16,
            base_low: (tss_base_high >> 16) as u16, 
            base_middle: 0, 
            access: 0, 
            granularity: 0, 
            base_high: 0, 
        };
        gdt
    };
);

pub unsafe fn init_gdt() {
    let gdt_ptr = &*GDT as *const [GDTEntry; 7];
    let ptr = GDTPointer {
        limit: (core::mem::size_of::<[GDTEntry; 7]>() - 1) as u16,
        base: gdt_ptr as u64,
    };

    unsafe {
        asm!(
            "lgdt ({ptr})",
            "pushq $0x08",
            "leaq 1f(%rip), {tmp}",
            "pushq {tmp}",
            "lretq",
            "1:",
            "movw $0x10, {tmp:x}",
            "movw {tmp:x}, %ds",
            "movw {tmp:x}, %es",
            "movw {tmp:x}, %ss",
            "movw $0, {tmp:x}",
            "movw {tmp:x}, %fs",
            "movw {tmp:x}, %gs",
            ptr = in(reg) &ptr,
            tmp = out(reg) _,
            options(att_syntax, readonly, nostack, preserves_flags)
        );

        asm!(
            "ltrw {sel:x}",
            sel = in(reg) 0x28_u16,
            options(att_syntax, nostack, preserves_flags)
        )
    }

    log_to_serial("GDT INIT OK\n");
}
