pub mod pit;

use core::sync::atomic::{AtomicUsize, Ordering};

pub static TICKS: AtomicUsize = AtomicUsize::new(0);

pub fn get_ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
}
