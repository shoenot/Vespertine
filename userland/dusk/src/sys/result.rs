use core::fmt::Display;

use vabi::{
    ProcInfo,
    ProcTermReason,
};
use vstd::prelude::*;

#[derive(Debug, Clone)]
pub enum ShellResult {
    None,
    InternalError(Error),
    Launched(ProcInfo),
    ChangeDirFail(String, Error),
    FailedToLaunch(String, Error),
    FailedToRender(String, Error),
    _AccessDenied(String),
    NotFound(String),
}

impl Display for ShellResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellResult::None => Ok(()),
            ShellResult::Launched(info) => format_process_result(info, f),
            ShellResult::InternalError(e) => write!(f, "internal error: {:?}", e),
            ShellResult::ChangeDirFail(path, err) => write!(f, "couldn't change dir to {}: {:?}", path, err),
            ShellResult::FailedToLaunch(name, err) => write!(f, "failed to launch {}: {:?}", name, err),
            ShellResult::FailedToRender(name, err) => write!(f, "failed to render {}: {:?}", name, err),
            ShellResult::_AccessDenied(name) => write!(f, "access denied: {}", name),
            ShellResult::NotFound(name) => write!(f, "not found: {}", name),
        }
    }
}

fn format_process_result(info: &ProcInfo, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    match info.term_reason {
        ProcTermReason::None => write!(
            f,
            "pid {} still running: state={:?}, threads={}, memory={} bytes",
            info.pid, info.state, info.active_threads, info.memory_usage
        ),
        ProcTermReason::Exited => write!(f, "pid {} exited with code {}", info.pid, info.term_code),
        ProcTermReason::Terminated => write!(f, "pid {} terminated with reason {}", info.pid, info.term_code),
        ProcTermReason::Faulted => write!(f, "pid {} faulted: {}, detail={:#x}", info.pid, info.fault_name(), info.term_detail),
    }
}
