use crate::kernel::acpi;
use crate::kernel::acpi::sdt::ACPISDTHeader;


#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct FADT {
    header: ACPISDTHeader,
    fw_ctrl: u32,
    dsdt: u32,
    _reserved: u8,
    preferred_pm_prof: u8,
    sci_interrupt: u16,
    smi_cmdport: u32,
    acpi_enable: u8,
    acpi_disable: u8,
    s4bios_req: u8,
    pstate_ctrl: u8,
    pm1a_event_blk: u32,
    pm1b_event_blk: u32,
    pm1a_ctrl_blk: u32,
    pm1b_ctrl_blk: u32,
    pm2_ctrl_blk: u32,
    pm_timer_blk: u32,
    gpe0_blk: u32,
    gpe1_blk: u32,
    pm1_event_len: u8,
    pm1_ctrl_len: u8,
    pm2_ctrl_len: u8,
    pm_timer_len: u8,
    gpe0_len: u8,
    gpe1_len: u8,
    gpe1_base: u8,
    cstate_ctrl: u8,
    worst_c2_latency: u16,
    worst_c3_latency: u16,
    flush_size: u16,
    flush_stride: u16,
    duty_offset: u8,
    duty_width: u8,
    day_alarm: u8,
    month_alarm: u8,
    century: u8,
    boot_arch_flags: u16,
    _reserved2: u8,
    flags: u32,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct FADTv2 {
    v1: FADT,
    reset_reg: GenericAddrStruct,
    reset_val: u8,
    _reserved_3: [u8; 3],
    x_fw_ctrl: u64,
    x_dsdt: u64,
    x_pm1a_event_blk: GenericAddrStruct,
    x_pm1b_event_blk: GenericAddrStruct,
    x_pm1a_ctrl_blk: GenericAddrStruct,
    x_pm1b_ctrl_blk: GenericAddrStruct,
    x_pm2_ctrl_blk: GenericAddrStruct,
    x_pm_timer_blk: GenericAddrStruct,
    x_gpe0_blk: GenericAddrStruct,
    x_gpe1_blk: GenericAddrStruct,
}

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct GenericAddrStruct {
    addr_space: u8,
    bit_width: u8,
    bit_offset: u8,
    access_size: u8,
    address: u64,
}

pub fn get_pm_timer_addr() -> (usize, bool) {
    let rsdp = acpi::rsdp::Rsdp::get();
    let sdt = acpi::sdt::SDTArray::get(rsdp.get_table());
    let fadt_addr = match sdt.find_table(b"FACP") {
        Some(addr) => addr,
        None => panic!("Couldn't find ACPI FADT table"),
    };

    let fadt_v1 = unsafe { &*(fadt_addr as *const FADT) };

    if fadt_v1.header.revision >= 2 {
        let fadt_v2 = unsafe { &*(fadt_addr as *const FADTv2) };
        if fadt_v2.x_pm_timer_blk.address != 0 {
            let is_mmio = if fadt_v2.x_pm_timer_blk.addr_space == 0 { true } else { false };
            return (fadt_v2.x_pm_timer_blk.address as usize, is_mmio);
        }
    }

    (fadt_v1.pm_timer_blk as usize, false)
}
