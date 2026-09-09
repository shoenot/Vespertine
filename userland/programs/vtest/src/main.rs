#![no_std]
#![no_main]
extern crate alloc;

mod test;
use vstd::prelude::*;

pub static HELP_TEXT: &'static str = "usage: vtest [test suite] [flags]";

#[vapp::main]
fn main(_pkg: &ProcessInitPackage) -> Result<(), Error> {
    let args = env::args();

    let Some(suite) = args.get(1) else {
        return Err(Error::invalid_argument(HELP_TEXT.into()));
    };

    let test_suite_args = &args[2..];

    match suite.as_str() {
        "input" => test::input_test(test_suite_args)?,
        _ => return Err(Error::invalid_argument(HELP_TEXT.into())),
    }

    Ok(())
}
