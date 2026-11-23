use vcli::args::{
    Command,
    Opt,
};
use vstd::HandleReader;
use vstd::prelude::*;

static WRITE_OPTIONS: &[Opt] = &[Opt::flag("help", Some('h'), Some("help")), Opt::value("output-file", Some('o'), Some("output-file"))];

pub fn run(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("write").options(WRITE_OPTIONS).parse(args).map_err(Error::from)?;

    if matches.flag("help") {
        return vstd::typed::write_help("usage: stream write -o <file> [text...]");
    }

    let Some(path_str) = matches.value("output-file") else {
        return Err(Error::invalid_argument("stream write needs a file to write to".into()));
    };

    let pathbuf = PathBuf::from_str(path_str);
    let path = pathbuf.as_path();

    let outfile = match File::open_with_rights(&path, AccessRights::WRITE) {
        Ok(file) => file,
        Err(e) if e.kind == ErrorKind::NotFound => File::create(&path)?,
        Err(e) => return Err(e),
    };

    let write_output = |bytes: &[u8]| {
        outfile.write_all(bytes).map_err(|error| {
            error.context(format_args!("cannot write '{}'", path.as_str()))
        })
    };

    if matches.positional_count() != 0 {
        let mut s = String::new();
        for (i, word) in matches.positionals().iter().enumerate() {
            if i > 0 {
                s.push_str(" ");
            }

            s.push_str(word);
        }
        write_output(s.as_bytes())?;
    } else {
        let reader = TypedReader::new(HandleReader::new(env::source()));
        let mut wrote_value = false;

        while let Some(value) = reader
            .next_value()
            .map_err(|error| error.context("cannot read typed input"))?
        {
            match value {
                ShellValue::Value(v) => {
                    if wrote_value {
                        write_output(b"\n")?;
                    }

                    let rendered = v.display_with(Default::default());
                    write_output(rendered.as_bytes())?;
                    wrote_value = true;
                }
                ShellValue::StreamEnd => break,
                ShellValue::RecordSchema { .. } | ShellValue::RecordPresentation { .. } => {}
                ShellValue::Error(message) => return Err(Error::application(message)),
                ShellValue::StreamIntent { .. } => {}
            }
        }
    };
    Ok(())
}
