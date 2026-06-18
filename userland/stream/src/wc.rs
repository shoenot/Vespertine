use alloc::string::String;
use vespertine_cli::args::Opt;
use vespertine_std::Error;

static WC_OPTIONS: &[Opt] = &[Opt::flag("lines-only", Some('l'), Some("lines-only"))];

pub fn run(_dummy: &[String]) -> Result<(), Error> {
    Ok(())
}
