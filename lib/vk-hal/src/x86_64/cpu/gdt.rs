use core::ptr::write_volatile;

use crate::x86_64::msr::{
    read_from_msr,
    write_to_msr,
};

pub const KERNEL_CS: u64 = 0x08;
pub const KERNEL_SS: u64 = 0x10;
pub const USER_SS: u64 = 0x18 | 3;
pub const USER_CS: u64 = 0x20 | 3;

const IA32_EFER: u32 = 0xC0000080;
const IA32_STAR: u32 = 0xC0000081;
const IA32_LSTAR: u32 = 0xC0000082;
const IA32_FMASK: u32 = 0xC0000084;

pub type BootAllocFn = fn(size: usize, align: usize) -> usize;

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_middle: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct GdtPointer {
    limit: u16,
    base: u64,
}

impl GdtEntry {
    const fn new(access: u8, flags: u8) -> Self {
        GdtEntry { limit_low: 0xFFFF, base_low: 0, base_middle: 0, access, granularity: flags | 0x0F, base_high: 0 }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct TaskStateSegment {
    reserved_1: u32,
    pub rsp: [u64; 3],
    reserved_2: u64,
    pub ist: [u64; 7],
    reserved_3: u64,
    reserved_4: u16,
    pub iomap_base: u16,
}

impl TaskStateSegment {
    fn new(alloc: BootAllocFn) -> Self {
        let mut tss = TaskStateSegment { 
            reserved_1: 0, 
            rsp: [0; 3], 
            reserved_2: 0, 
            ist: [0; 7], 
            reserved_3: 0, 
            reserved_4: 0, 
            iomap_base: 104 
        };
        let int_stack_ptr = alloc(8192, 4096);
        let stack_top = int_stack_ptr as u64 + 8192;
    
        tss.rsp[0] = stack_top;
        tss
    }
}

fn get_gdt_template() -> [GdtEntry; 7] {
    [
        GdtEntry { limit_low: 0, base_low: 0, base_middle: 0, access: 0, granularity: 0, base_high: 0 },
        GdtEntry::new(0x9A, 0xA0), // kernel code
        GdtEntry::new(0x92, 0xA0), // kernel data
        GdtEntry::new(0xF2, 0xA0), // user data
        GdtEntry::new(0xFA, 0xA0), // user code
        GdtEntry::new(0, 0),       // tss
        GdtEntry::new(0, 0),       // tss
    ]
}

#[allow(dead_code)]
#[repr(C, packed)]
struct TSSDescriptor {
    low: GdtEntry,
    high_base: u32,
    _reserved: u32,
}

pub struct CpuLocalGdt {
    pub gdt: [GdtEntry; 7],
    pub tss: TaskStateSegment,
    pub gdt_ptr: GdtPointer,
}

unsafe extern "sysv64" {
    fn _syscall_entry();
}

pub fn init_syscall_msrs() {
    unsafe {
        // EFER = current with bit 0 enabled to activate syscall/sysret
        let efer = read_from_msr(IA32_EFER);
        write_to_msr(efer | 1, IA32_EFER);

        // STAR = low 32 = 0; 32-47 = kernel base selector; 48-63 = user base selector;
        let kernel_base_selector = 0x08 | 0;
        let user_base_selector = 0x10 | 3;
        let hi = (user_base_selector << 16) as u32 | kernel_base_selector;
        write_to_msr((hi as u64) << 32, IA32_STAR);

        // LSTAR = asm syscall entry trampoline yippee
        write_to_msr(_syscall_entry as *const () as u64, IA32_LSTAR);

        // IA32_FMASK = interrupt flags, direction flag, nested task, resume flag
        write_to_msr(0x200 | 0x400 | 0x4000 | 0x10000, IA32_FMASK);
    }
}

pub fn init_core_gdt(gdt_ptr: *mut CpuLocalGdt, alloc: BootAllocFn) {
    unsafe {
        write_volatile(&mut (*gdt_ptr).gdt, get_gdt_template());
        write_volatile(&mut (*gdt_ptr).tss, TaskStateSegment::new(alloc));

        let tss_ptr = &mut (*gdt_ptr).tss as *mut TaskStateSegment;
        let tss_base = tss_ptr as usize;
        let tss_limit = (size_of::<TaskStateSegment>() - 1) as u16;
        let tss_base_high = (tss_base >> 32) as u32;

        (*gdt_ptr).gdt[5] = GdtEntry {
            limit_low: tss_limit,
            base_low: tss_base as u16,
            base_middle: (tss_base >> 16) as u8,
            access: 0x89,
            granularity: 0,
            base_high: (tss_base >> 24) as u8,
        };

        (*gdt_ptr).gdt[6] = GdtEntry {
            limit_low: tss_base_high as u16,
            base_low: (tss_base_high >> 16) as u16,
            base_middle: 0,
            access: 0,
            granularity: 0,
            base_high: 0,
        };

        (*gdt_ptr).gdt_ptr = GdtPointer {
            limit: (size_of::<[GdtEntry; 7]>() - 1) as u16,
            base: &mut (*gdt_ptr).gdt as *mut [GdtEntry; 7] as u64,
        };
    }
}
