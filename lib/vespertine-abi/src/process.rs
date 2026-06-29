use core::slice;
extern crate alloc;

use crate::{
    CapabilityGrant,
    HandleID,
    PROC_NAME_LEN_MAX,
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

pub const PROC_FAULT_PAGE: u32 = 1;
pub const PROC_FAULT_GENERAL_PROTECTION: u32 = 2;
pub const PROC_FAULT_INVALID_OPCODE: u32 = 3;

#[repr(C)]
#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub pid: usize,
    pub user: UserID,

    pub name_len: u16,
    pub reserved: u16,
    pub name: [u8; PROC_NAME_LEN_MAX],

    pub state: ProcState,
    pub active_threads: usize,
    pub memory_usage: usize,

    pub term_reason: ProcTermReason,
    pub term_code: u32,
    pub term_detail: usize,
}

impl ProcInfo {
    pub fn zeroed() -> Self {
        Self {
            pid: 0,
            user: UserID(0),

            name_len: 0,
            reserved: 0,
            name: [0; PROC_NAME_LEN_MAX],

            state: ProcState::Running,
            active_threads: 0,
            memory_usage: 0,

            term_reason: ProcTermReason::None,
            term_code: 0,
            term_detail: 0,
        }
    }

    pub fn name(&self) -> &str {
        let len = core::cmp::min(self.name_len as usize, self.name.len());
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    pub fn set_name(&mut self, value: &str) {
        let len = core::cmp::min(value.len(), self.name.len());

        self.name_len = len as u16;
        self.name[..len].copy_from_slice(&value.as_bytes()[..len]);
    }

    pub fn fault_name(&self) -> &'static str {
        match self.term_code {
            PROC_FAULT_PAGE => "page fault",
            PROC_FAULT_GENERAL_PROTECTION => "general protection fault",
            PROC_FAULT_INVALID_OPCODE => "invalid_opcode",
            _ => "unknown fault",
        }
    }

    pub fn successful(&self) -> bool { self.term_reason == ProcTermReason::Exited && self.term_code == 0 }

    pub fn status_code(&self) -> u32 {
        match self.term_reason {
            ProcTermReason::None => 0,
            ProcTermReason::Exited => self.term_code,
            ProcTermReason::Terminated => 128 + self.term_code,
            ProcTermReason::Faulted => 128,
        }
    }

    pub fn short_status(&self) -> &'static str {
        match self.term_reason {
            ProcTermReason::None => "running",
            ProcTermReason::Exited => {
                if self.term_code == 0 {
                    "ok"
                } else {
                    "exited"
                }
            }
            ProcTermReason::Terminated => "terminated",
            ProcTermReason::Faulted => "faulted",
        }
    }
}
