use crate::cpu::{
    BootAllocFn,
    KernelCorePtr,
};
use crate::x86_64::boot::{ApplicationProcessor, ApplicationProcessorEntry, for_each_ap, start_ap};
use crate::x86_64::cpu::init_cpu_local_with_hardware_id;

pub type ApplicationCoreEntry = ApplicationProcessorEntry;
pub type KernelCoreAllocator = fn(logical_id: usize) -> KernelCorePtr;
pub type KernelCoreRegistrar = fn(logical_id: usize, kernel_core: KernelCorePtr);

pub fn start_application_cores(
    allocate_kernel_core: KernelCoreAllocator,
    register_kernel_core: KernelCoreRegistrar,
    entry: ApplicationCoreEntry,
    alloc: BootAllocFn,
) -> usize {
    let mut logical_id = 1;
    for_each_ap(|ap: ApplicationProcessor| {
        let kernel_core = allocate_kernel_core(logical_id);
        let cpu_local = init_cpu_local_with_hardware_id(ap.hardware_id as usize, logical_id, kernel_core, alloc);

        register_kernel_core(logical_id, kernel_core);
        start_ap(ap, entry, cpu_local);
        logical_id += 1;
    });
    logical_id
}
