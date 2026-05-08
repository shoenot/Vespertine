use super::{rsdp::Rsdp, sdt};

pub fn get_io_apic_addr() -> usize {
    let rsdp = Rsdp::get();
    let table = rsdp.get_table();
    let sdt 
}
