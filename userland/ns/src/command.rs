use alloc::string::String;
use vespertine_abi::shell::{RECORD_PRESENTATION_DEFAULT, ValueType};
use vespertine_cli::args::{Command, Opt};
use vespertine_rt::println;
use vespertine_std::{Error, HandleWriter, env, fs::{Dir, EntryKind, File, Path}, shell::TypedWriter};

static LIST_OPTIONS: &[Opt] = &[
    Opt::flag("all", Some('a'), None),
    Opt::flag("help", Some('h'), Some("help")),
];

const NS_DIR_ENTRY_SCHEMA: u64 = 1;

pub fn list(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("list")
        .options(LIST_OPTIONS)
        .parse(args)
        .map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: ns list [flags] [dir]");
        return Ok(());
    }

    if matches.positional_count() > 1 {
        return Err(Error::invalid_argument("usage: ns list [flags] [dir]".into()));
    }

    let dir = if let Some(path) = matches.positional(0) {
        Dir::open(&Path::new(path))?
    } else {
        Dir::open(&Path::new("."))?
    };

    let out = TypedWriter::new(HandleWriter::new(env::sink()));

    out.record_schema(
        NS_DIR_ENTRY_SCHEMA, 
        &[
            ("name", ValueType::String),
            ("kind", ValueType::String),
        ],
    )?;

    out.record_presentation(
        NS_DIR_ENTRY_SCHEMA, 
        RECORD_PRESENTATION_DEFAULT, 
        &[0],
    )?;

    let mut dir_iter = dir.list()?;

    while let Some(entry) = dir_iter.next() {
        if entry.name == "lost+found" {
            continue;
        }

        if entry.name.starts_with('.') {
            if !matches.flag("all") {
                continue;
            }
        }

        out.record(
            NS_DIR_ENTRY_SCHEMA, 
            &[
                entry.name.as_str(), 
                match entry.kind {
                    EntryKind::File => "File",
                    EntryKind::Directory => "Directory",
                    EntryKind::Object => "Object",
                }
            ],
        )?;
    }

    out.record_end(NS_DIR_ENTRY_SCHEMA)?;

    Ok(())
}

pub fn mkdir(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("mkdir")
        .parse(args)
        .map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: ns mkdir [flags] [dir]");
    }

    if matches.positional_count() == 0 {
        println!("usage: ns mkdir [flags] [dir]");
    }

    for name in matches.positionals().iter() {
        Dir::create_dir(&Path::new(name))?;
    }

    Ok(())
}

pub fn touch(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("touch")
        .parse(args)
        .map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: ns touch [flags] [dir]");
    }

    if matches.positional_count() == 0 {
        println!("usage: ns touch [flags] [dir]");
    }

    for name in matches.positionals().iter() {
        File::create(&Path::new(name))?;
    }

    Ok(())
}

pub fn delete(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("delete")
        .parse(args)
        .map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: ns delete [flags] [dir]");
    }

    if matches.positional_count() == 0 {
        println!("usage: ns delete [flags] [dir]");
    }

    for name in matches.positionals().iter() {
        Dir::remove(&Path::new(name))?;
    }

    Ok(())
}
