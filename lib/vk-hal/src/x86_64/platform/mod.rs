mod acpi;

extern crate alloc;
use alloc::vec::Vec;
use common::once::KernelOnceCell as OnceCell;

use crate::x86_64::apic::lapic::init_apic_direct_map; 
use acpi::load_platform_info;

pub static PLATFORM_INFO: OnceCell<PlatformInfo> = OnceCell::new();
pub static PLATFORM_HOOKS: OnceCell<PlatformInit> = OnceCell::new();

#[derive(Clone, Copy)]
pub struct IoApicInfo {
    pub id: u8,
    pub address: usize,
    pub gsi_base: u32,
}

#[derive(Clone, Copy)]
pub struct InterruptOverride {
    pub bus: u8,
    pub source: u8,
    pub gsi: u32,
    pub flags: u16,
}

pub struct PlatformInfo {
    pub hpet_base: Option<usize>,
    pub century_register: u8,
    pub ioapics: Vec<IoApicInfo>,
    pub interrupt_overrides: Vec<InterruptOverride>,
}

pub struct PlatformInit {
    pub rsdp_addr: usize,
    pub direct_map_offset: usize,
    pub map_mmio: fn(phys: u64, size: usize) -> Option<usize>,
}

pub fn init_early(init: PlatformInit) {
    PLATFORM_HOOKS.get_or_init(|| init);
    init_apic_direct_map(platform_hooks().direct_map_offset);
}

pub fn init() {
    PLATFORM_INFO.get_or_init(|| {
        load_platform_info(platform_hooks().rsdp_addr, platform_hooks().direct_map_offset)
    });
}

fn platform_info() -> &'static PlatformInfo {
    PLATFORM_INFO.get().expect("HAL platform info was not initialized")
}

fn platform_hooks() -> &'static PlatformInit {
    PLATFORM_HOOKS.get().expect("HAL platform hooks were not initialized")
}

pub(crate) fn map_mmio(phys: u64, size: usize) -> Option<usize> {
    (platform_hooks().map_mmio)(phys, size)
}

pub(crate) fn hpet_base() -> Option<usize> {
    platform_info().hpet_base
}

pub(crate) fn century_register() -> u8 {
    platform_info().century_register
}

pub(crate) fn ioapics() -> &'static [IoApicInfo] {
    &platform_info().ioapics
}

pub(crate) fn interrupt_overrides() -> &'static [InterruptOverride] {
    &platform_info().interrupt_overrides
}

pub(crate) fn interrupt_override_for_source(source: u8) -> Option<InterruptOverride> {
    platform_info().interrupt_overrides.iter().copied()
        .find(|entry| entry.source == source)
}
