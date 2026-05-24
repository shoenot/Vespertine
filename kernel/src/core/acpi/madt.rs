#![allow(dead_code)]

use alloc::vec::Vec;
use core::ptr::read_unaligned;

use crate::core::acpi::sdt::{
    ACPISDTHeader,
    SDTArray,
};

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct MADT {
    pub header: ACPISDTHeader,
    pub local_apic_address: u32,
    pub flags: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct MADTEntryHeader {
    pub entry_type: u8,
    pub length: u8,
}

// Type 0
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct ProcLocalApicEntry {
    pub header: MADTEntryHeader,
    pub acpi_proc_id: u8,
    pub apic_id: u8,
    pub flags: u32,
}

// Type 1
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct IoApicEntry {
    pub header: MADTEntryHeader,
    pub io_apic_id: u8,
    pub reserved: u8,
    pub io_apic_addr: u32,
    pub global_system_interrupt_base: u32,
}

// Type 2
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct InterruptSourceOverrrideEntry {
    pub header: MADTEntryHeader,
    pub bus: u8,
    pub source: u8,
    pub global_system_interrupt: u32,
    pub flags: u16,
}

pub struct LocalApic {
    pub proc_id: u8,
    pub apic_id: u8,
    pub is_enabled: bool,
}

pub struct IoApic {
    pub id: u8,
    pub addr: u32,
    pub gsi_base: u32,
}

pub struct InterruptOverride {
    pub bus: u8,
    pub source: u8,
    pub gsi: u32,
    pub flags: u16,
}

pub struct MadtInfo {
    pub local_apic_base: u32,
    pub local_apics: Vec<LocalApic>,
    pub io_apics: Vec<IoApic>,
    pub overrides: Vec<InterruptOverride>,
}

pub fn parse_madt(sdt_array: &SDTArray) -> MadtInfo {
    let madt_addr = match sdt_array.find_table(b"APIC") {
        Some(addr) => addr,
        None => panic!("MADT not found"),
    };

    let madt = unsafe { &*(madt_addr as *const MADT) };

    let mut info =
        MadtInfo { local_apic_base: madt.local_apic_address, local_apics: Vec::new(), io_apics: Vec::new(), overrides: Vec::new() };

    let total_length = madt.header.length as usize;
    let mut current_addr = madt_addr + size_of::<MADT>();
    let end_addr = madt_addr + total_length;

    while current_addr < end_addr {
        let entry_header = unsafe { read_unaligned(current_addr as *const MADTEntryHeader) };
        let entry_type = entry_header.entry_type;
        let entry_length = entry_header.length as usize;

        match entry_type {
            0 => {
                let entry = unsafe { read_unaligned(current_addr as *const ProcLocalApicEntry) };
                let is_enabled = (entry.flags & 1) != 0;
                info.local_apics.push(LocalApic { proc_id: entry.acpi_proc_id, apic_id: entry.apic_id, is_enabled });
            }
            1 => {
                let entry = unsafe { read_unaligned(current_addr as *const IoApicEntry) };
                info.io_apics.push(IoApic { id: entry.io_apic_id, addr: entry.io_apic_addr, gsi_base: entry.global_system_interrupt_base });
            }
            2 => {
                let entry = unsafe { read_unaligned(current_addr as *const InterruptSourceOverrrideEntry) };
                info.overrides.push(InterruptOverride {
                    bus: entry.bus,
                    source: entry.source,
                    gsi: entry.global_system_interrupt,
                    flags: entry.flags,
                })
            }
            _ => {}
        }

        current_addr += entry_length;
    }
    info
}
