use alloc::string::String;
use vespertine_abi::{AccessRights, tag::CAP_APP_TERMCTRL};
use vespertine_rt::println;
use vespertine_std::{Error, ErrorKind, Exec, env, term::{check_raw_mode, clear_term_screen, unset_raw_mode}};

use crate::lexer::ShellError;

pub fn launch_command(name: &str, args: &[String]) {
    let res = build_exec(name, args).and_then(Exec::spawn);

    match res {
        Ok(proc) => {
            match proc.wait() {
                Ok(exit) => println!("Process exited: {:?}", exit),
                Err(error) => println!("[ERROR] {}: {:?}", name, error),
            }
        }
        Err(error) if error.kind == ErrorKind::NotFound => {
            println!("Command not found: {}", name);
        }
        Err(error) => {
            println!("[ERROR] Could not launch {}: {:?}", name, error);
        }
    }

    let _ = unset_raw_mode();
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
