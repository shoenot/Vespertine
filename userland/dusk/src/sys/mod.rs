use alloc::string::{String, ToString};
use vespertine_abi::{AccessRights, ProcessExitInfo, tag::CAP_APP_TERMCTRL};
use vespertine_rt::println;
use vespertine_std::{Error, ErrorKind, Exec, env, term::{check_raw_mode, clear_term_screen, unset_raw_mode}};

use crate::{error::ShellError, runtime::env::ShellContext};

pub fn launch_command(name: &str, args: &[String], context: &ShellContext) -> Result<ProcessExitInfo, ShellError> {
    let res = build_exec(name, args).and_then(Exec::spawn);
    let info = match res {
        Ok(proc) => {
            match proc.wait() {
                Ok(exit) => Ok(exit),
                Err(error) => Err(ShellError::LaunchError(name.to_string(), error)),
            }
        },
        Err(error) if error.kind == ErrorKind::NotFound => {
            Err(ShellError::NotFound(name.to_string()))
        },
        Err(error) => Err(ShellError::LaunchError(name.to_string(), error)),
    };

    let _ = unset_raw_mode();
    info
}

pub fn build_exec(name: &str, args: &[String]) -> Result<Exec, Error> {
    let exec = Exec::new(name.into())
        .source(env::source())
        .sink(env::sink())
        .args(args)
        .root_rights(
            AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE | AccessRights::EXECUTE,
        );

    match name {
        "kilo" => exec.grant(CAP_APP_TERMCTRL, AccessRights::READ | AccessRights::WRITE),
        _ => Ok(exec),
    }
}
