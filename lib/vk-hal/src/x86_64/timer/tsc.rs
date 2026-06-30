use core::arch::x86_64::_rdtsc;

#[derive(Copy, Clone, Debug)]
pub(crate) struct Tsc {
    pub(crate) frequency: usize,
}

impl Tsc {
    pub(crate) fn read_counter(&self) -> usize {
        read_tsc_counter()
    }
}

pub(crate) fn read_tsc_counter() -> usize {
    unsafe { _rdtsc() as usize }
}
