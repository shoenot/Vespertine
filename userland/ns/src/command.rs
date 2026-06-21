use alloc::string::String;

use vespertine_abi::typed::{
    DATETIME_HAS_OFFSET,
    DateTimeValue,
    FileSizeValue,
    ValueType,
};
use vespertine_cli::args::{
    Command,
    Opt,
};
use vespertine_rt::println;
use vespertine_std::Error;
use vespertine_std::fs::{
    Dir,
    EntryKind,
    File,
    Path,
    PathBuf,
    stat,
};
use vespertine_std::typed::{
    RecordStream,
    TypedValue,
};

static LIST_OPTIONS: &[Opt] = &[Opt::flag("all", Some('a'), None), Opt::flag("help", Some('h'), Some("help"))];

const NS_DIR_ENTRY_SCHEMA: u64 = 1;

pub fn list(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("list").options(LIST_OPTIONS).parse(args).map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: ns list [flags] [dir]");
        return Ok(());
    }

    if matches.positional_count() > 1 {
        return Err(Error::invalid_argument("usage: ns list [flags] [dir]".into()));
    }

    let dir_path = if let Some(path) = matches.positional(0) { Path::new(path) } else { Path::new(".") };

    let dir = Dir::open(&dir_path)?;

    let mut out = RecordStream::typed_default_out(
        NS_DIR_ENTRY_SCHEMA,
        &[
            ("name", ValueType::String),
            ("kind", ValueType::String),
            ("size", ValueType::FileSize),
            ("owner", ValueType::Integer),
            ("mode", ValueType::Integer),
            ("created", ValueType::DateTime),
            ("modified", ValueType::DateTime),
        ],
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

        out.row_values(&[
            TypedValue::String(entry.name),
            TypedValue::String(String::from(kind_str)),
            TypedValue::FileSize(FileSizeValue {
                bytes: stat.size as i128,
                block_size: stat.block_size as u64,
                blocks: stat.blocks as i128,
                flags: 0,
                reserved: 0,
            }),
            TypedValue::Integer(stat.user as i128),
            TypedValue::Integer(stat.mode as i128),
            TypedValue::DateTime(DateTimeValue {
                unix_seconds: stat.ctime_sec,
                nanos: stat.ctime_nsec as u32,
                offset_minutes: 0,
                flags: DATETIME_HAS_OFFSET,
                calendar: 0,
                reserved: 0,
            }),
            TypedValue::DateTime(DateTimeValue {
                unix_seconds: stat.mtime_sec,
                nanos: stat.mtime_nsec as u32,
                offset_minutes: 0,
                flags: DATETIME_HAS_OFFSET,
                calendar: 0,
                reserved: 0,
            }),
        ])?;
    }

    out.finish()?;

    Ok(())
}

pub fn ns_stat(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("stat").parse(args).map_err(Error::from)?;
    if matches.flag("help") {
        println!("usage: ns stat [path]");
    }

    if matches.positional_count() == 0 {
        println!("usage: ns stat [path]");
    }

    if matches.positional_count() > 1 {
        println!("usage: ns stat [path]");
    }

    let stat_path = Path::new(matches.positionals()[0]);
    let stat_info = stat(&stat_path)?;

    let mut out = RecordStream::typed_default_out(
        NS_DIR_ENTRY_SCHEMA,
        &[
            ("name", ValueType::String),
            ("kind", ValueType::String),
            ("size", ValueType::FileSize),
            ("owner", ValueType::Integer),
            ("mode", ValueType::Integer),
            ("created", ValueType::DateTime),
            ("modified", ValueType::DateTime),
        ],
        &["name"],
    )?;

    out.table(&["kind", "size", "owner", "mode", "created", "modified"])?;

    let kind_str = match stat_info.object_type {
        0 => "File",
        1 => "Directory",
        2 => "Object",
        _ => "",
    };

    out.row_values(&[
        TypedValue::String(String::from(kind_str)),
        TypedValue::FileSize(FileSizeValue {
            bytes: stat_info.size as i128,
            block_size: stat_info.block_size as u64,
            blocks: stat_info.blocks as i128,
            flags: 0,
            reserved: 0,
        }),
        TypedValue::Integer(stat_info.user as i128),
        TypedValue::Integer(stat_info.mode as i128),
        TypedValue::DateTime(DateTimeValue {
            unix_seconds: stat_info.ctime_sec,
            nanos: stat_info.ctime_nsec as u32,
            offset_minutes: 0,
            flags: DATETIME_HAS_OFFSET,
            calendar: 0,
            reserved: 0,
        }),
        TypedValue::DateTime(DateTimeValue {
            unix_seconds: stat_info.mtime_sec,
            nanos: stat_info.mtime_nsec as u32,
            offset_minutes: 0,
            flags: DATETIME_HAS_OFFSET,
            calendar: 0,
            reserved: 0,
        }),
    ])?;

    out.finish()?;

    Ok(())
}

pub fn mkdir(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("mkdir").parse(args).map_err(Error::from)?;

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
    let matches = Command::new("touch").parse(args).map_err(Error::from)?;

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
    let matches = Command::new("delete").parse(args).map_err(Error::from)?;

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
