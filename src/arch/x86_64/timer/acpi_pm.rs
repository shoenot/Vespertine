use core::ptr::{read_volatile, write_volatile};
use crate::HHDMOFFSET;
use crate::kernel::time::ClockSource;
use crate::arch::x86_64::io::{inl, outl};
use crate::kernel::acpi::fadt::get_pm_timer_addr;

#[derive(Copy, Clone, Debug)]
pub struct ACPI_PM_Timer {
    pub timer_addr: usize,
    pub is_mmio: bool,
}

impl ClockSource for ACPI_PM_Timer {
    fn name(&self) -> &'static str {
        "ACPI_PM_TIMER"
    }

    fn read_counter(&self) -> usize {
        unsafe {
            if self.is_mmio {
                    let ptr = (self.timer_addr + *HHDMOFFSET) as *mut u32;
                    (read_volatile(ptr) as u32 & 0x00FF_FFFF) as usize 
            } else {
                (inl(self.timer_addr as u16) & 0x00FF_FFFF) as usize
            }
        }
    }

    fn frequency(&self) -> usize {
        3_579_545
    }
}
