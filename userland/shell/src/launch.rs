use alloc::{string::String, vec::Vec};
use vespertine_abi::{AccessRights, app::termios::Termios, tag::{TAG_APP_TERM, TAG_SYS_CLOCK, TAG_SYS_SOCKFAC}};
use vespertine_rt::println;
use vespertine_std::{
    Error, ErrorKind, Exec, env, term::{set_terminfo, unset_raw_mode}
};

pub fn launch_command(name: &str, args: &[String]) {
    let res = build_exec(name, args).and_then(Exec::spawn);

    match res {
        Ok(proc) => {
            if let Err(error) = proc.wait() {
                println!("[ERROR] {}: {:?}", name, error);
            }
        },
        Err(error) if error.kind == ErrorKind::NotFound => {
            println!("Command not found: {}", name);
        },
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
            AccessRights::READ |
            AccessRights::WRITE |
            AccessRights::CREATE |
            AccessRights::EXECUTE
        );

    match name {
        "dt" => exec.grant(TAG_SYS_CLOCK, AccessRights::READ),
        "kilo" | "cat" => exec.grant(TAG_APP_TERM, AccessRights::READ | AccessRights::WRITE),
        "ns" => exec.grant(TAG_SYS_SOCKFAC, AccessRights::CREATE),
        _ => Ok(exec),
    }
}
