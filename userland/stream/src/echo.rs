use alloc::string::String;
use vespertine_cli::args::{Command, Opt};
use vespertine_rt::{print, println};
use vespertine_std::Error;

static ECHO_OPTIONS: &[Opt] = &[
    Opt::flag("no-newline", Some('n'), Some("no-newline")),
    Opt::flag("help", Some('h'), Some("help")),
];

pub fn run(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("echo")
        .options(ECHO_OPTIONS)
        .parse(args)
        .map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: stream echo [-n/--no-newline] [text..]");
        return Ok(());
    }

    for (i, word) in matches.positionals().iter().enumerate() {
        if i > 0 {
            print!(" ");
        }

        print!("{}", word);
    }

    if !matches.flag("no-newline") {
        println!("");
    }

    Ok(())
}
