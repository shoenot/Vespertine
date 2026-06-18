use core::slice;

use crate::{CapabilityGrant, HandleID, UserID};

pub const AT_VESPERTINE_INITPKG: usize = 0x6fff_0001;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProcessInitPackage {
    pub self_handle: HandleID,
    pub root_handle: HandleID,
    pub source_handle: HandleID,
    pub sink_handle: HandleID,
    pub memory_pool_handle: HandleID,
    pub cwd_handle: HandleID,

    pub capabilities_ptr: *const CapabilityGrant,
    pub capabilities_len: usize,

    pub argc: usize,
    pub argv: *const *const u8,
    pub envp: *const *const u8,
}

impl ProcessInitPackage {
    pub fn capabilities(&self) -> &[CapabilityGrant] {
        unsafe { slice::from_raw_parts(self.capabilities_ptr, self.capabilities_len) }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProcStatus {
    pub pid: usize,
    pub user: UserID,
    pub active_threads: usize,
    pub is_terminated: bool,
    pub memory_usage: usize,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessExitKind {
    Running = 0,
    Exited = 1,
    Killed = 2,
    Faulted = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProcessExitInfo {
    pub kind: ProcessExitKind,
    pub code: u32,
    pub detail: u64,
}

impl ProcessExitInfo {
    pub const fn running() -> Self {
        Self {
            kind: ProcessExitKind::Running,
            code: 0,
            detail: 0,
        }
    }

    pub const fn exited(code: u32) -> Self {
        Self {
            kind: ProcessExitKind::Exited,
            code,
            detail: 0,
        }
    }

    pub const fn killed(reason: u32) -> Self {
        Self {
            kind: ProcessExitKind::Killed,
            code: reason,
            detail: 0,
        }
    }
}
