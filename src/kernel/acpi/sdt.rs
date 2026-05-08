use alloc::{slice, vec::Vec};
use core::slice::from_raw_parts;
use crate::HHDMOFFSET;
use super::rsdp::AcpiRoot;

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct ACPISDTHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

pub struct SDTArray {
    pub header: ACPISDTHeader,
    pub sdt_addresses: Vec<usize>,
}

impl SDTArray {
    fn get(acpi_root: AcpiRoot) -> Self {
        match acpi_root {
            AcpiRoot::RSDT(addr) => {
                let header_ptr = addr as *const ACPISDTHeader;
                unsafe {
                    let header = *header_ptr;
                    let len = (header.length as usize - size_of::<ACPISDTHeader>()) / size_of::<u32>();
                    let array_ptr = (addr + size_of::<ACPISDTHeader>()) as *const u32;
                    let sdt_addresses = from_raw_parts(array_ptr, len)
                        .iter()
                        .map(|&ptr| ptr as usize + *HHDMOFFSET)
                        .collect();

                    SDTArray { header, sdt_addresses }
                }
            },
            AcpiRoot::XSDT(addr) => {
                let header_ptr = addr as *const ACPISDTHeader;
                unsafe {
                    let header = *header_ptr;
                    let len = (header.length as usize - size_of::<ACPISDTHeader>()) / size_of::<u64>();
                    let array_ptr = (addr + size_of::<ACPISDTHeader>()) as *const u64;
                    let sdt_addresses = from_raw_parts(array_ptr, len)
                        .iter()
                        .map(|&ptr| ptr as usize + *HHDMOFFSET)
                        .collect();

                    SDTArray { header, sdt_addresses }
                }
            },
        }
    }
}
