pub mod fadt;
pub mod hpet;
pub mod madt;
pub mod rsdp;
pub mod sdt;

extern crate alloc;
use alloc::vec::Vec;

use crate::x86_64::platform::{
    InterruptOverride,
    IoApicInfo,
    PlatformInfo,
};

pub fn load_platform_info(rsdp_addr: usize, direct_map_offset: usize) -> PlatformInfo {
    let rsdp = rsdp::Rsdp::get(rsdp_addr);
    let sdt = sdt::SDTArray::get(rsdp.get_table(direct_map_offset), direct_map_offset);
    let madt = madt::parse_madt(&sdt);

    PlatformInfo {
        hpet_base: hpet::get_hpet_base_addr(&sdt),
        century_register: fadt::get_century_register(&sdt),
        ioapics: madt
            .io_apics
            .iter()
            .map(|entry| IoApicInfo {
                id: entry.id,
                address: entry.addr as usize,
                gsi_base: entry.gsi_base,
            })
            .collect(),
        interrupt_overrides: madt
            .overrides
            .iter()
            .map(|entry| InterruptOverride {
                bus: entry.bus,
                source: entry.source,
                gsi: entry.gsi,
                flags: entry.flags,
            })
            .collect(),
    }
}
