use alloc::collections::btree_map::BTreeMap;
use alloc::format;
use alloc::string::String;

use vespertine_abi::{
    AccessRights,
    HandleID, ProcTermReason,
};
use vespertine_rt::syscall::sys_close;
use vespertine_std::fs::{
    PathBuf,
    resolve_from,
};
use vespertine_std::{
    Error,
    env,
};

use crate::sys::ShellResult;

pub struct ShellContext {
    cwd: PathBuf,
    cwd_handle: HandleID,
    environment: BTreeMap<String, String>,
    pub last_result: ShellResult,
}

impl ShellContext {
    pub fn new() -> Self {
        Self { cwd: PathBuf::root(), cwd_handle: env::cwd(), environment: BTreeMap::new(), last_result: ShellResult::None }
    }

    pub fn cwd(&self) -> &PathBuf { &self.cwd }

    pub fn cwd_handle(&self) -> HandleID { self.cwd_handle }

    pub fn change_dir(&mut self, path: PathBuf) -> Result<(), Error> {
        let new_handle = resolve_from(&path.as_path(), env::root(), self.cwd_handle, AccessRights::TRAVERSE | AccessRights::LIST)?;
        let new_display_path = self.cwd.join(&path.as_path());

        if self.cwd_handle != env::cwd() {
            let _ = sys_close(self.cwd_handle);
        }

        self.cwd_handle = new_handle;
        self.cwd = new_display_path;
        Ok(())
    }

    pub fn status(&self) -> String {
        match self.last_result {
            ShellResult::Launched(info) => {
                if info.successful() {
                    "(0)".into()
                } else {
                    format!("({}: {})", info.status_code(), info.short_status())
                }
            },
            ShellResult::None => format!(""),
            _ => format!("(err)"),
        }
    }


    pub fn last_details(&self) -> String {
        match self.last_result {
            ShellResult::Launched(info) => {
                match info.term_reason {
                    ProcTermReason::None => format!(
                        "pid: {}, state: {:?}, threads: {}, memory: {} bytes",
                        info.pid,
                        info.state,
                        info.active_threads,
                        info.memory_usage
                    ),
                    ProcTermReason::Exited => format!(
                        "pid: {}, exited with code {}",
                        info.pid,
                        info.term_code
                    ),
                    ProcTermReason::Terminated => format!(
                        "pid: {}, terminated with reason {}",
                        info.pid,
                        info.term_code
                    ),
                    ProcTermReason::Faulted => format!(
                        "pid: {}, faulted: {}, detail: {:#x}",
                        info.pid,
                        info.fault_name(),
                        info.term_detail
                    ),
                }
            },
            _ => format!(""),
        }
    }

    pub fn last_success(&self) -> bool {
        match self.last_result {
            ShellResult::Launched(info) => info.successful(),
            ShellResult::None => true,
            _ => false,
        }
    }
}
