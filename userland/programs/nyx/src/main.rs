#![no_std]
#![no_main]

mod command;

extern crate alloc;
use vstd::prelude::*;

pub static HELP_TEXT: &'static str = "usage: nyx [flags] [command]";

#[vapp::main]
fn main(_pkg: &ProcessInitPackage) -> Result<(), Error> {
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
