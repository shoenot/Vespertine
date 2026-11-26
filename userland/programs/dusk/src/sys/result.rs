use core::fmt::Display;

use vespertine_abi::{
    ProcInfo, ProcSignal, ProcTermReason,
};
use vstd::prelude::*;

#[derive(Debug, Clone)]
pub enum ShellResult {
    None,
    InternalError(Error),
    Launched(ProcInfo),
    ChangeDirFail(String, Error),
    FailedToLaunch(String, Error),
    FailedToWait(String, Error),
    FailedToRender(String, Error),
    _AccessDenied(String),
    NotFound(String),
    CommandError(String, Error),
}

impl Display for ShellResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellResult::None => Ok(()),
            ShellResult::Launched(info) => format_process_result(info, f),
            ShellResult::InternalError(error) => write!(f, "internal error: {error}"),
            ShellResult::ChangeDirFail(path, error) => write!(f, "cannot change directory to '{path}': {error}"),
            ShellResult::FailedToLaunch(name, error) => write!(f, "cannot launch '{name}': {error}"),
            ShellResult::FailedToWait(name, error) => write!(f, "cannot wait for '{name}': {error}"),
            ShellResult::FailedToRender(name, error) => write!(f, "cannot render '{name}': {error}"),
            ShellResult::_AccessDenied(name) => write!(f, "access denied: {}", name),
            ShellResult::NotFound(name) => write!(f, "not found: {}", name),
            ShellResult::CommandError(name, err) => write!(f, "{name}: {err}"),
        }
    }
}

impl ShellResult {
    pub fn from_render_error(name: &str, error: Error) -> Self {
        match error.kind {
            ErrorKind::Application => Self::CommandError(name.into(), error),
            _ => Self::FailedToRender(name.into(), error),
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
        ProcTermReason::TerminatedBySignal => {
            let signal = ProcSignal::from_raw(info.term_code)
                .map(|signal| signal.name())
                .unwrap_or("unknown signal");
            write!(f, "pid: {} terminated by {} ({}) from pid {}", info.pid, signal, info.term_code, info.term_detail)
        }
    }
}
