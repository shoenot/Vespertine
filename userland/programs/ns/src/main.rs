#![no_std]
#![no_main]
mod command;

extern crate alloc;
use vstd::prelude::*;

const HELP_TEXT: &str = concat!(
    "usage: ns [command] [flags] [args]\n", 
    "commands:\n",
    "  help\n",
    "  list\n",
    "  mkdir\n",
    "  touch\n",
    "  delete\n",
);

#[vapp::main]
fn main(_pkg: &ProcessInitPackage) -> Result<(), Error> {
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
        "help" | "--help" | "-h" => return vstd::typed::write_help(HELP_TEXT),
        _ => return Err(Error::invalid_argument(HELP_TEXT.into())),
    }
    Ok(())
}
