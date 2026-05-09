use core::arch::asm;
use crate::kernel::time::ClockSource;

#[derive(Copy, Clone, Debug)]
pub struct TSC {
    pub frequency: usize,
}

impl ClockSource for TSC {
    fn name(&self) -> &'static str {
        "TSC"
    }

    fn read_counter(&self) -> usize {
        let mut lo: u32;
        let mut hi: u32;
        unsafe {
            asm!(
                "rdtsc",
                out("eax") lo,
                out("edx") hi,
                options(nomem, nostack)
            );
        }
        (hi as usize) << 32 | lo as usize
    }

    fn frequency(&self) -> usize {
        self.frequency
    }
}
