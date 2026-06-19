use alloc::string::String;
use vespertine_cli::args::{Command, Opt};
use vespertine_rt::{println};
use vespertine_std::{Error, typed::{TypedValue, TypedWriter}};

static ECHO_OPTIONS: &[Opt] = &[
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

    let mut s = String::new();
    for (i, word) in matches.positionals().iter().enumerate() {
        if i > 0 {
            s.push_str(" ");
        }

        s.push_str(word);
    }

    let out = TypedWriter::out();
    out.value(&TypedValue::String(s))?;
    out.stream_end()
}
