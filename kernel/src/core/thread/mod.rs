pub mod dispatch;
pub mod idle;
pub mod priority;
pub mod reap;
pub mod schedule;
pub mod tcb;
pub mod wait;
pub mod workqueue;

use core::alloc::LayoutError;
use core::fmt;

pub use tcb::*;

#[derive(Debug)]
pub enum ThreadError {
    SpawnAllocationError(LayoutError),
    AllocationFailed,
}

impl fmt::Display for ThreadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnAllocationError(layout) => write!(f, "Allocation Error; Thread spawn failed.\nDetails: {:?}", layout),
            Self::AllocationFailed => write!(f, "Allocation Error; Memory allocation returned null."),
        }
    }
}

impl From<LayoutError> for ThreadError {
    fn from(value: LayoutError) -> Self { ThreadError::SpawnAllocationError(value) }
}
