pub mod schedule;
pub mod tcb;
pub mod idle;
pub mod wait;
pub mod cpu;

use core::{
    alloc::LayoutError,
    fmt,
};

pub use tcb::*;

#[derive(Debug)]
pub enum ThreadError {
    SpawnAllocationError(LayoutError),
}

impl fmt::Display for ThreadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnAllocationError(layout) => write!(f, "Allocation Error; Thread spawn failed.\nDetails: {:?}", layout),
        }
    }
}

impl From<LayoutError> for ThreadError {
    fn from(value: LayoutError) -> Self { ThreadError::SpawnAllocationError(value) }
}
