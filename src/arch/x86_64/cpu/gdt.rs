use core::ptr::write_volatile;

use crate::BOOTSTRAP_ALLOC;

pub(crate) const KERNEL_CS: u64 = 0x08;
pub(crate) const KERNEL_SS: u64 = 0x10;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct GDTEntry {
    limit_low: u16,
    base_low: u16,
    base_middle: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub(crate) struct GDTPointer {
    limit: u16,
    base: u64,
}

impl GDTEntry {
    const fn new(access: u8, flags: u8) -> Self {
        GDTEntry { limit_low: 0xFFFF, base_low: 0, base_middle: 0, access, granularity: flags | 0x0F, base_high: 0 }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct TaskStateSegment {
    reserved_1: u32,
    rsp: [u64; 3],
    reserved_2: u64,
    ist: [u64; 7],
    reserved_3: u64,
    reserved_4: u16,
    iomap_base: u16,
}

impl TaskStateSegment {
    fn new() -> Self {
        let mut tss =
            TaskStateSegment { reserved_1: 0, rsp: [0; 3], reserved_2: 0, ist: [0; 7], reserved_3: 0, reserved_4: 0, iomap_base: 104 };
        let int_stack_ptr = BOOTSTRAP_ALLOC.lock().alloc(8192, 4096);
        let stack_top = int_stack_ptr as u64 + 8192;

        tss.rsp[0] = stack_top;
        tss
    }
}

fn get_gdt_template() -> [GDTEntry; 7] {
    [
        GDTEntry { limit_low: 0, base_low: 0, base_middle: 0, access: 0, granularity: 0, base_high: 0 },
        GDTEntry::new(0x9A, 0xA0),
        GDTEntry::new(0x92, 0xA0),
        GDTEntry::new(0xFA, 0xA0),
        GDTEntry::new(0xF2, 0xA0),
        GDTEntry::new(0, 0), // for tss
        GDTEntry::new(0, 0), // for tss
    ]
}

#[allow(dead_code)]
#[repr(C, packed)]
struct TSSDescriptor {
    low: GDTEntry,
    high_base: u32,
    _reserved: u32,
}

pub struct CPULocalGDT {
    pub gdt: [GDTEntry; 7],
    pub tss: TaskStateSegment,
    pub gdt_ptr: GDTPointer,
}

pub(in crate::arch::x86_64) fn init_core_gdt(lgdt_ptr: *mut CPULocalGDT) {
    unsafe {
        write_volatile(&mut (*lgdt_ptr).gdt, get_gdt_template());
        write_volatile(&mut (*lgdt_ptr).tss, TaskStateSegment::new());

        let tss_ptr = &mut (*lgdt_ptr).tss as *mut TaskStateSegment;
        let tss_base = tss_ptr as usize;
        let tss_limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u16;
        let tss_base_high = (tss_base >> 32) as u32;

        (*lgdt_ptr).gdt[5] = GDTEntry {
            limit_low: tss_limit,
            base_low: tss_base as u16,
            base_middle: (tss_base >> 16) as u8,
            access: 0x89,
            granularity: 0,
            base_high: (tss_base >> 24) as u8,
        };

        (*lgdt_ptr).gdt[6] = GDTEntry {
            limit_low: tss_base_high as u16,
            base_low: (tss_base_high >> 16) as u16,
            base_middle: 0,
            access: 0,
            granularity: 0,
            base_high: 0,
        };

        (*lgdt_ptr).gdt_ptr = GDTPointer {
            limit: (core::mem::size_of::<[GDTEntry; 7]>() - 1) as u16,
            base: &mut (*lgdt_ptr).gdt as *mut [GDTEntry; 7] as u64,
        };
    }
}
