use alloc::string::String;

use vespertine_abi::AccessRights;
use vespertine_cli::args::{
    Command,
    Opt,
};
use vespertine_rt::println;
use vespertine_std::fs::{
    File,
    PathBuf,
};
use vespertine_std::typed::{
    ShellValue,
    TypedReader,
};
use vespertine_std::{
    Error,
    HandleReader,
    Write,
    env,
};

static WRITE_OPTIONS: &[Opt] = &[Opt::flag("help", Some('h'), Some("help")), Opt::value("output-file", Some('o'), Some("output-file"))];

pub fn run(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("write").options(WRITE_OPTIONS).parse(args).map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: stream echo -o [file] [text..]");
        return Ok(());
    }

    let Some(path_str) = matches.value("output-file") else {
        return Err(Error::invalid_argument("stream write needs a file to write to".into()));
    };

    let pathbuf = PathBuf::from_str(path_str);
    let outfile = match File::open_with_rights(&pathbuf.as_path(), AccessRights::WRITE) {
        Ok(file) => file,
        Err(_) => File::create(&pathbuf.as_path())?,
    };

    if matches.positional_count() != 0 {
        let mut s = String::new();
        for (i, word) in matches.positionals().iter().enumerate() {
            if i > 0 {
                s.push_str(" ");
            }

            s.push_str(word);
        }
        outfile.write_all(s.as_bytes())?;
    } else {
        let reader = TypedReader::new(HandleReader::new(env::source()));
        let mut wrote_value = false;

        while let Some(value) = reader.next_value()? {
            match value {
                ShellValue::Value(v) => {
                    if wrote_value {
                        outfile.write_all(b"\n")?;
                    }

                    let rendered = v.display_with(Default::default());
                    outfile.write_all(rendered.as_bytes())?;
                    wrote_value = true;
                }
                ShellValue::StreamEnd => break,
                ShellValue::RecordSchema { .. } | ShellValue::RecordPresentation { .. } => {}
                ShellValue::Error(message) => return Err(Error::invalid_argument(message)),
            }
        }
    };
    Ok(())
}
