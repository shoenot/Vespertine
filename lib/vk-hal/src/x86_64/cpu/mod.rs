use core::arch::asm;

pub mod cpuid;
pub mod gdt;

pub use gdt::{
    BootAllocFn,
    CpuLocalGdt,
    KERNEL_CS,
    KERNEL_SS,
    USER_CS,
    USER_SS,
    init_core_gdt,
    init_syscall_msrs,
};

use crate::x86_64::fpu::init_fpu;
use crate::x86_64::apic::lapic::LocalApicDriver;
use crate::x86_64::cpu::gdt::GdtPointer;
use crate::x86_64::apic::lapic::{
    LocalApic,
    init_local_apic,
    register_cpu_local,
};
use crate::x86_64::msr::write_to_msr;

pub type KernelCorePtr = *mut ();
pub type CpuLocalPtr = *mut ();

const KERNEL_GS_BASE: u32 = 0xC0000101;

#[repr(C)]
pub(crate) struct CpuLocalData {
    pub self_ptr: *mut CpuLocalData, // offset 0x00
    pub saved_user_rsp: usize,      // offset 0x08
    pub kernel_rsp: usize,          // offset 0x10
    pub kernel_core: KernelCorePtr, // offset 0x18
    pub logical_id: usize,
    pub hardware_id: usize,
    pub core_gdt: CpuLocalGdt,
    pub local_apic: LocalApic,
}

unsafe extern "sysv64" {
    fn load_gdt(ptr: &GdtPointer);
}

pub fn init_bootstrap_cpu_local(logical_id: usize, kernel_core: KernelCorePtr, alloc: BootAllocFn) -> CpuLocalPtr {
    unsafe {
        let data_addr = alloc(size_of::<CpuLocalData>(), align_of::<CpuLocalData>());
        let data_ptr = data_addr as *mut CpuLocalData;

        let local_apic = init_local_apic();
        let hardware_id = local_apic.id() as usize;

        let gdt_ptr = &mut (*data_ptr).core_gdt as *mut CpuLocalGdt;
        init_core_gdt(gdt_ptr, alloc);

        (*data_ptr).self_ptr = data_ptr;
        (*data_ptr).saved_user_rsp = 0;
        (*data_ptr).kernel_rsp = 0;
        (*data_ptr).kernel_core = kernel_core;
        (*data_ptr).logical_id = logical_id;
        (*data_ptr).hardware_id = hardware_id;
        (*data_ptr).local_apic = local_apic;

        register_cpu_local(logical_id, data_ptr);

        data_ptr as CpuLocalPtr
    }
}


pub fn init_cpu_local_with_hardware_id(hardware_id: usize, logical_id: usize, kernel_core: KernelCorePtr, alloc: BootAllocFn) -> CpuLocalPtr {
    unsafe {
        let data_addr = alloc(size_of::<CpuLocalData>(), align_of::<CpuLocalData>());
        let data_ptr = data_addr as *mut CpuLocalData;

        let local_apic = init_local_apic();

        let gdt_ptr = &mut (*data_ptr).core_gdt as *mut CpuLocalGdt;
        init_core_gdt(gdt_ptr, alloc);

        (*data_ptr).self_ptr = data_ptr;
        (*data_ptr).saved_user_rsp = 0;
        (*data_ptr).kernel_rsp = 0;
        (*data_ptr).kernel_core = kernel_core;
        (*data_ptr).logical_id = logical_id;
        (*data_ptr).hardware_id = hardware_id;
        (*data_ptr).local_apic = local_apic;

        register_cpu_local(logical_id, data_ptr);

        data_ptr as CpuLocalPtr
    }
}

pub fn activate_core(cpu_local: CpuLocalPtr) {
    unsafe {
        let data_ptr = cpu_local as *mut CpuLocalData;
        let gdt_ptr = (*data_ptr).core_gdt.gdt_ptr;
        load_gdt(&gdt_ptr);

        let data_addr = data_ptr as usize;

        write_to_msr(data_addr as u64, 0xC000_0100);
        write_to_msr(data_addr as u64, KERNEL_GS_BASE);

        init_syscall_msrs();
    }
}

pub fn init_bsp_state(alloc: BootAllocFn) {
    init_fpu(true, alloc);
}

pub fn init_ap_state(alloc: BootAllocFn) {
    init_fpu(false, alloc);
}

pub fn kernel_core_from_cpu_local(cpu_local: CpuLocalPtr) -> KernelCorePtr {
    unsafe {
        (*(cpu_local as *mut CpuLocalData)).kernel_core
    }
}

#[inline(always)]
pub(crate) fn current_cpu_local() -> *mut CpuLocalData {
    let ptr: usize;
    unsafe {
        asm!(
            "mov {}, gs:[0]",
            out(reg) ptr,
            options(nomem, nostack, preserves_flags),
        );
    }
    ptr as *mut CpuLocalData
}

#[inline(always)]
pub fn current_kernel_core() -> KernelCorePtr {
    unsafe {
        (*current_cpu_local()).kernel_core
    }
}

#[inline(always)]
pub fn current_logical_id() -> usize {
    unsafe {
        (*current_cpu_local()).logical_id
    }
}

#[inline(always)]
pub fn current_hardware_id() -> usize {
    unsafe {
        (*current_cpu_local()).hardware_id
    }
}

#[inline(always)]
pub unsafe fn set_kernel_stack(stack_top: usize) {
    unsafe {
        let local = &mut *current_cpu_local();
        local.kernel_rsp = stack_top;
        local.core_gdt.tss.rsp[0] = stack_top as u64;
    }
}

#[inline(always)]
pub fn halt() {
    unsafe {
        asm!("hlt", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn idle() {
    unsafe {
        asm!("sti; hlt", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn set_user_thread_pointer(ptr: usize) {
    unsafe {
        write_to_msr(ptr as u64, 0xC000_0100);
    }
}

pub fn halt_loop() -> ! {
    loop {
        halt();
    }
}

