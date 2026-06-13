use core::fmt::Display;

use alloc::string::{String, ToString};
use vespertine_abi::{AccessRights, ProcessExitInfo, tag::CAP_APP_TERMCTRL};
use vespertine_rt::println;
use vespertine_std::{Error, ErrorKind, Exec, env, term::{check_raw_mode, clear_term_screen, unset_raw_mode}};

use crate::{error::ShellError, runtime::env::ShellContext};

#[derive(Debug, Clone)]
pub enum ShellResult {
    None,
    Launched(ProcessExitInfo),
    ChangeDirFail(String, Error),
    FailedToLaunch(String, Error),
    AccessDenied(String),
    NotFound(String),
}

impl Display for ShellResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellResult::None => core::fmt::Result::Ok(()),
            ShellResult::Launched(ei) => write!(f, "exit info: {:?}", ei),
            ShellResult::ChangeDirFail(path, err) => write!(f, "couldn't change dir to {}: {:?}", path, err),
            ShellResult::FailedToLaunch(name, err) => write!(f, "failed to launch {}: {:?}", name, err),
            ShellResult::AccessDenied(name) => write!(f, "access denied: {}", name),
            ShellResult::NotFound(name) => write!(f, "not found: {}", name),
        }
    }
}

pub fn launch_command(name: &str, args: &[String], context: &ShellContext) -> ShellResult {
    let res = build_exec(name, args, context).and_then(Exec::spawn);
    let info = match res {
        Ok(proc) => {
            match proc.wait() {
                Ok(exit) => ShellResult::Launched(exit),
                Err(error) => ShellResult::FailedToLaunch(name.to_string(), error),
            }
        },
        Err(error) if error.kind == ErrorKind::NotFound => {
            ShellResult::NotFound(name.to_string())
        },
        Err(error) => ShellResult::FailedToLaunch(name.to_string(), error),
    };

    let _ = unset_raw_mode();
    info
}

pub fn build_exec(name: &str, args: &[String], context: &ShellContext) -> Result<Exec, Error> {
    let cwd_rights = AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE | AccessRights::EXECUTE;
    let exec = Exec::new(name.into())
        .source(env::source())
        .sink(env::sink())
        .cwd(context.cwd_handle(), cwd_rights)
        .args(args)
        .root_rights(
            AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE | AccessRights::EXECUTE,
        );

    match name {
        "kilo" => exec.grant(CAP_APP_TERMCTRL, AccessRights::READ | AccessRights::WRITE),
        _ => Ok(exec),
    }
}
