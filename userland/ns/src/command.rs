use alloc::{format, string::String, vec::Vec};
use vespertine_abi::shell::{RECORD_PRESENTATION_DEFAULT, ValueType};
use vespertine_cli::args::{Command, Opt};
use vespertine_rt::println;
use vespertine_std::{Error, HandleWriter, env, fs::{Dir, EntryKind, File, Path, PathBuf, stat}, shell::{RecordStream, TypedWriter}};

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

    let dir_path = if let Some(path) = matches.positional(0) {
        Path::new(path)
    } else {
        Path::new(".")
    };

    let dir = Dir::open(&dir_path)?;

    let mut out = RecordStream::default_out(
        NS_DIR_ENTRY_SCHEMA, 
        &["name", "kind", "size", "owner", "mode", "created", "modified"],
        &["name"],
    )?;
    out.table(&["name", "kind", "size", "owner", "mode", "created", "modified"])?;

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

        let kind_str = match entry.kind {
            EntryKind::File => "File",
            EntryKind::Directory => "Directory",
            EntryKind::Object => "Object",
        };

        let entry_path = PathBuf::from(dir_path.as_str()).join(&Path::new(entry.name.as_str()));
        let stat = stat(&entry_path.as_path())?;

        let size = format!("{}", stat.size);
        let owner = format!("{}", stat.user);
        let mode = format!("{}", stat.mode);
        let creation_time = format!("{}", stat.ctime_sec);
        let modification_time = format!("{}", stat.mtime_sec);

        out.row(&[ entry.name.as_str(), kind_str, size.as_str(), owner.as_str(), mode.as_str(), creation_time.as_str(), modification_time.as_str() ])?;
    }

    out.finish()?;

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
