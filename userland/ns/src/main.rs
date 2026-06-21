#![no_std]
#![no_main]
mod command;

extern crate alloc;

use alloc::format;

use vespertine_abi::ProcessInitPackage;
use vespertine_rt::syscall::sys_close;
use vespertine_std::typed::TypedWriter;
use vespertine_std::{
    Error,
    env,
};

static HELP_TEXT: &'static str = "usage: ns [command] [flags] [args]\n
                                  commands:
                                  \tlist\n
                                  \tmkdir\n
                                  \ttouch\n
                                  \tdelete\n";

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        let out = TypedWriter::out();
        let _ = out.error(&*format!("ns error: {:?}", e));
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
        "list" => command::list(command_args)?,
        "mkdir" => command::mkdir(command_args)?,
        "touch" => command::touch(command_args)?,
        "delete" => command::delete(command_args)?,
        _ => return Err(Error::invalid_argument(HELP_TEXT.into())),
    }

    Ok(())
}
