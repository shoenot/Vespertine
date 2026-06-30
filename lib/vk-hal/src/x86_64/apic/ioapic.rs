use core::ptr::{
    read_volatile,
    write_volatile,
};

use common::lock::TicketLock;
use crate::x86_64::apic::lapic::hardware_id_for_core;
use crate::x86_64::platform;

const IOREGSEL_OFFSET: usize = 0x00;
const IOWIN_OFFSET: usize = 0x10;
const IOREDTBL_BASE: u8 = 0x10;

static IO_APIC: TicketLock<IOApic> = TicketLock::new(IOApic { base_addr: 0, gsi_base: 0 });

pub struct IOApic {
    pub(crate) base_addr: usize,
    pub(crate) gsi_base: usize,
}

impl IOApic {
    pub(crate) fn init(&mut self, phys_addr: usize, gsi_base: usize) {
        let virt_addr = platform::map_mmio(phys_addr as u64, 4096).expect("failed to map IOAPIC MMIO");
        self.base_addr = virt_addr;
        self.gsi_base = gsi_base;
    }

    unsafe fn write_reg(&self, reg: u8, value: u32) {
        let sel_ptr = (self.base_addr + IOREGSEL_OFFSET) as *mut u32;
        let win_ptr = (self.base_addr + IOWIN_OFFSET) as *mut u32;
        unsafe {
            write_volatile(sel_ptr, reg as u32);
            write_volatile(win_ptr, value);
        }
    }

    unsafe fn read_reg(&self, reg: u8) -> u32 {
        let sel_ptr = (self.base_addr + IOREGSEL_OFFSET) as *mut u32;
        let win_ptr = (self.base_addr + IOWIN_OFFSET) as *mut u32;
        unsafe {
            write_volatile(sel_ptr, reg as u32);
            read_volatile(win_ptr)
        }
    }

    pub(crate) fn mask_all(&self) {
        let version_reg = unsafe { self.read_reg(0x01) };
        let max_entry = ((version_reg >> 16) & 0xFF) as u8;
        for i in 0..=max_entry {
            let low_idx = IOREDTBL_BASE + (i * 2);
            let high_idx = IOREDTBL_BASE + (i * 2) + 1;
            unsafe {
                self.write_reg(low_idx, 1 << 16);
                self.write_reg(high_idx, 0);
            }
        }
    }

    pub(crate) fn set_entry(&self, gsi: u32, vector: u8, lapic_id: u32, masked: bool, active_high: bool, edge_triggered: bool) {
        if gsi < self.gsi_base as u32 {
            return;
        }

        let rel_gsi = (gsi - self.gsi_base as u32) as u8;
        let low_idx = IOREDTBL_BASE + (rel_gsi * 2);
        let high_idx = IOREDTBL_BASE + (rel_gsi * 2) + 1;

        // Bits 0-7: Vector
        // Bit 13: Interrupt pin polarity (0: Active High, 1: Active Low)
        // Bit 15: Trigger mode (0: Edge triggered, 1: Level triggered)
        // Bit 16: Mask (1 = Disabled, 0 = Enabled)
        let mut low_val = vector as u32;
        if masked {
            low_val |= 1 << 16;
        }
        if !active_high {
            low_val |= 1 << 13;
        }
        if !edge_triggered {
            low_val |= 1 << 15;
        }

        // Bits 56-63 (Shifted): Destination LAPIC ID
        let high_val = lapic_id << 24;

        unsafe {
            self.write_reg(low_idx, low_val);
            self.write_reg(high_idx, high_val);
        }
    }
}

pub fn init_ioapic() {
    let ioapic = platform::ioapics().first().expect("No IOAPIC found");
    let mut controller = IO_APIC.lock();
    controller.init(ioapic.address, ioapic.gsi_base as usize);
    controller.mask_all();
}

pub fn route_isa_irq(source: u8, vector: u8, target_core: usize) {
    let mut gsi = source as u32;
    let mut active_high = true;
    let mut edge_triggered = true;

    if let Some(entry) = platform::interrupt_override_for_source(source) {
        gsi = entry.gsi;

        if entry.flags & 0b11 == 0b11 {
            active_high = false;
        }

        if entry.flags & 0b1100 == 0b1100 {
            edge_triggered = false;
        }
    }

    let target_hardware_id = hardware_id_for_core(target_core) as u32;

    IO_APIC.lock().set_entry(gsi, vector, target_hardware_id, false, active_high, edge_triggered);
}
