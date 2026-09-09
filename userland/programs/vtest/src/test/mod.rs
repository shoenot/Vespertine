use alloc::string::String;
use vcli::args::Command;
use vstd::prelude::*;

pub fn input_test(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("input").parse(args).map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: vtest input [flags]");
    }

    Ok(())
}
