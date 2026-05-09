use crate::kernel::acpi;
use crate::kernel::acpi::sdt::ACPISDTHeader;

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct HPET {
    pub header: ACPISDTHeader,
    pub event_timer_blkid: u32,
    pub reserved: u32,
    pub address: u64,
    pub id: u8,
    pub min_ticks: u16,
    pub page_protection: u8,
}

pub fn get_hpet_base_addr() -> Option<usize> {
    let rsdp = acpi::rsdp::Rsdp::get();
    let sdt = acpi::sdt::SDTArray::get(rsdp.get_table());
    unsafe {
        let hpet_base_addr = match sdt.find_table(b"HPET") {
            Some(addr) => {
                let hpet_table = &*(addr as *const HPET);
                Some(hpet_table.address as usize)
            },
            None => None
        };
        hpet_base_addr
    }
}
