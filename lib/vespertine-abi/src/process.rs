use core::{slice, sync::atomic::AtomicU8};

use crate::{
    CapabilityGrant,
    HandleID,
    UserID,
};

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
    pub fn capabilities(&self) -> &[CapabilityGrant] { unsafe { slice::from_raw_parts(self.capabilities_ptr, self.capabilities_len) } }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcState {
    Running = 0,
    Terminating = 1,
    Terminated = 2,
    Suspended = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcTermReason {
    None = 0,
    Exited = 1,
    Terminated = 2,
    Faulted = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProcInfo {
    pub pid: usize,
    pub user: UserID,
    pub state: ProcState,
    pub active_threads: usize,
    pub memory_usage: usize,

    pub term_reason: ProcTermReason,
    pub term_code: u32,
    pub term_detail: usize,
}
