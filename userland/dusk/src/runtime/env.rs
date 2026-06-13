use alloc::{collections::btree_map::BTreeMap, format, string::{String, ToString}};
use vespertine_abi::{AccessRights, HandleID, ProcessExitInfo};
use vespertine_rt::syscall::sys_close;
use vespertine_std::{Error, env, fs::walk_path_from};

use crate::sys::ShellResult;

use super::ShellPath;

pub struct ShellContext {
    cwd: ShellPath,
    cwd_handle: HandleID,
    environment: BTreeMap<String, String>,
    pub last_result: ShellResult,
}

impl ShellContext {
    pub fn new() -> Self {
        Self {
            cwd: ShellPath::new("/"),
            cwd_handle: env::cwd(),
            environment: BTreeMap::new(),
            last_result: ShellResult::None,
        }
    }

    pub fn cwd(&self) -> &ShellPath {
        &self.cwd
    }

    pub fn cwd_handle(&self) -> HandleID {
        self.cwd_handle
    }

    pub fn change_dir(&mut self, path: ShellPath) -> Result<(), Error> {
        let path_string = path.to_string();

        let new_handle = walk_path_from(&path_string, env::root(), self.cwd_handle, AccessRights::READ)?;
        let new_display_path = self.cwd.join(&path);

        if self.cwd_handle != env::cwd() {
            let _ = sys_close(self.cwd_handle);
        }

        self.cwd_handle = new_handle;
        self.cwd = new_display_path;
        Ok(())
    }

    pub fn status(&self) -> String {
        match self.last_result {
            ShellResult::Launched(info) => format!("({})", info.code),
            ShellResult::None => format!(""),
            _ => format!("(err)")
        }
    }

    pub fn last_details(&self) -> String {
        match self.last_result {
            ShellResult::Launched(info) => format!("{:?}, code: {}, details: {}", info.kind, info.code, info.detail),
            _ => format!(""),
        }
    }

    pub fn last_success(&self) -> bool {
        match self.last_result {
            ShellResult::Launched(info) => if info.code == 0 { true } else { false },
            ShellResult::None => true,
            _ => false,
        }
    }
}

