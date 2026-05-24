use core::arch::x86_64::_rdtsc;

use crate::core::time::ClockSource;

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
pub(crate) struct TSC {
    pub(crate) frequency: usize,
}

impl ClockSource for TSC {
    fn name(&self) -> &'static str { "TSC" }

    fn read_counter(&self) -> usize { read_tsc_direct() }

    fn frequency(&self) -> usize { self.frequency }
}

pub(crate) fn read_tsc_direct() -> usize { unsafe { _rdtsc() as usize } }
