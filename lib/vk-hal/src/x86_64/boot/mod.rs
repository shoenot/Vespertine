use crate::cpu::CpuLocalPtr;

mod limine;

#[derive(Clone, Copy, Debug)]
pub struct BootFramebuffer {
    pub virtual_address: usize,
    pub physical_address: usize,
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootMemoryKind {
    Usable,
    BootloaderReclaimable,
    ExecutableAndModules,
    Reserved,
    Other,
}

#[derive(Clone, Copy, Debug)]
pub struct BootMemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: BootMemoryKind,
}

pub fn check() -> bool {
    limine::check()
}

pub fn direct_map_offset() -> usize {
    limine::direct_map_offset()
}

pub fn framebuffer() -> Option<BootFramebuffer> {
    limine::framebuffer()
}

pub fn for_each_memory_region(f: impl FnMut(BootMemoryRegion)) {
    limine::for_each_memory_region(f);
}

pub(crate) fn acpi_rsdp_addr() -> usize {
    limine::acpi_rsdp_addr()
}

pub(crate) type ApplicationProcessorEntry = extern "C" fn(CpuLocalPtr) -> !;

#[derive(Clone, Copy)]
pub(crate) struct ApplicationProcessor {
    pub hardware_id: usize,
}

pub(crate) fn for_each_ap(mut f: impl FnMut(ApplicationProcessor)) {
    limine::for_each_ap(|processor| f(processor));
}

pub(crate) fn start_ap(processor: ApplicationProcessor, entry: ApplicationProcessorEntry, cpu_local:
CpuLocalPtr) {
    limine::start_ap(processor, entry, cpu_local);
}
