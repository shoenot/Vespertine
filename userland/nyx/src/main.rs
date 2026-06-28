#![no_std]
#![no_main]

mod command;

extern crate alloc;
use alloc::format;
use vespertine_abi::ProcessInitPackage;
use vespertine_rt::syscall::sys_close;
use vespertine_std::{Error, env, typed::TypedWriter};

pub static HELP_TEXT: &'static str = "usage: nyx [flags] [command]";

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        let out = TypedWriter::out();
        let _ = out.error(&*format!("sys error: {:?}", e));
        let _ = out.stream_end();
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let args = env::args();

    let Some(command) = args.get(1) else {
        return Err(Error::invalid_argument(HELP_TEXT.into()));
    };

    let command_args = &args[2..];

    match command.as_str() {
        "i" | "installed" => command::installed(command_args)?,
        "I" | "info" => command::info(command_args)?,
        _ => return Err(Error::invalid_argument(HELP_TEXT.into())),
    }

    Ok(())
}
