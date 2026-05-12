#![allow(dead_code)]

use alloc::vec::Vec;
use core::slice::from_raw_parts;

use super::rsdp::AcpiRoot;
use crate::memory::HHDMOFFSET;

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct ACPISDTHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

pub struct SDTArray {
    pub header: ACPISDTHeader,
    pub sdt_addresses: Vec<usize>,
}

impl SDTArray {
    pub fn get(acpi_root: AcpiRoot) -> Self {
        match acpi_root {
            AcpiRoot::RSDT(addr) => {
                let header_ptr = addr as *const ACPISDTHeader;
                unsafe {
                    let header = *header_ptr;
                    let len = (header.length as usize - size_of::<ACPISDTHeader>()) / size_of::<u32>();
                    let array_ptr = (addr + size_of::<ACPISDTHeader>()) as *const u32;
                    let sdt_addresses = from_raw_parts(array_ptr, len).iter().map(|&ptr| ptr as usize + *HHDMOFFSET).collect();

                    SDTArray { header, sdt_addresses }
                }
            }
            AcpiRoot::XSDT(addr) => {
                let header_ptr = addr as *const ACPISDTHeader;
                unsafe {
                    let header = *header_ptr;
                    let len = (header.length as usize - size_of::<ACPISDTHeader>()) / size_of::<u64>();
                    let array_ptr = (addr + size_of::<ACPISDTHeader>()) as *const u64;
                    let sdt_addresses = from_raw_parts(array_ptr, len).iter().map(|&ptr| ptr as usize + *HHDMOFFSET).collect();

                    SDTArray { header, sdt_addresses }
                }
            }
        }
    }

    pub fn find_table(&self, signature: &[u8; 4]) -> Option<usize> {
        for &sdt_addr in &self.sdt_addresses {
            let header = unsafe { &*(sdt_addr as *const ACPISDTHeader) };
            if &header.signature == signature {
                return Some(sdt_addr);
            }
        }
        None
    }
}
