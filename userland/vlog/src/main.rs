#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use core::str;

use vespertine_abi::{
    AccessRights,
    ProcessInitPackage,
};
use vespertine_common::datetime::epoch_to_datetime;
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_std::clock::Time;
use vespertine_std::fs::{
    Dir,
    File,
    Path,
    resolve,
};
use vespertine_std::log::LogReader;
use vespertine_std::{
    Error,
    ErrorKind,
    Read,
    Write,
};

const LOG_ROOT: &str = "/System/Logs";
const LOG_BOOT_ROOT: &str = "/System/Logs/Boots";
const MAX_RECORD_BYTES: usize = 4096;
const RETAIN_BOOT_COUNT: usize = 8;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };

    if let Err(error) = run() {
        println!("vlog failed: {:?}", error);
    }

    let _ = sys_close(pkg.sink_handle);
}

struct AppendFile {
    file: File,
    offset: usize,
}

impl AppendFile {
    fn open(path: &str) -> Result<Self, Error> {
        let path = Path::new(path);
        let file = match File::open_with_rights(&path, AccessRights::WRITE) {
            Ok(file) => file,
            Err(error) if error.kind == ErrorKind::NotFound => File::create(&path)?,
            Err(error) => return Err(error),
        };
        let offset = file.stat()?.size as usize;
        Ok(Self { file, offset })
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.file.write_at(bytes, self.offset)?;
        self.offset += bytes.len();
        Ok(())
    }
}

fn run() -> Result<(), Error> {
    println!("vlog starting");

    ensure_dir(LOG_ROOT)?;
    ensure_dir(LOG_BOOT_ROOT)?;

    let boot_id = boot_id();
    let boot_dir = format!("{}/{}", LOG_BOOT_ROOT, boot_id);
    let process_dir = format!("{}/processes", boot_dir);

    ensure_dir(&boot_dir)?;
    ensure_dir(&process_dir)?;
    prune_old_boots()?;

    let log_handle = resolve(&Path::new("/System/Services/Log"), AccessRights::READ)?;
    let reader = LogReader::from_handle(log_handle);
    let mut index = AppendFile::open(&format!("{}/index.vlog", boot_dir))?;
    let mut processes = BTreeMap::<String, AppendFile>::new();
    let mut buf = [0u8; MAX_RECORD_BYTES];

    println!("vlog online: {}", boot_id);

    loop {
        let len = reader.read(&mut buf)?;
        if len == 0 {
            continue;
        }

        index.append(&buf[..len])?;

        if let Some(path) = process_log_path(&process_dir, &buf[..len]) {
            if !processes.contains_key(&path) {
                let file = AppendFile::open(&path)?;
                processes.insert(path.clone(), file);
            }

            if let Some(file) = processes.get_mut(&path) {
                file.append(&buf[..len])?;
            }
        }
    }
}

fn boot_id() -> String {
    let (seconds, _) = Time::now();
    let dt = epoch_to_datetime(seconds as i64);
    format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second)
}

fn ensure_dir(path: &str) -> Result<(), Error> {
    match resolve(&Path::new(path), AccessRights::TRAVERSE | AccessRights::LIST) {
        Ok(handle) => {
            sys_close(handle).map_err(Error::from)?;
            Ok(())
        },
        Err(error) if error.kind == ErrorKind::NotFound => Dir::create_dir(&Path::new(path)).map(|_| ()),
        Err(error) => Err(error),
    }
}

fn prune_old_boots() -> Result<(), Error> {
    let dir = Dir::open(&Path::new(LOG_BOOT_ROOT))?;
    let mut boots = alloc::vec::Vec::new();

    for entry in dir.list()? {
        boots.push(entry.name);
    }

    boots.sort();

    while boots.len() > RETAIN_BOOT_COUNT {
        let old = boots.remove(0);
        let path = format!("{}/{}", LOG_BOOT_ROOT, old);
        let _ = remove_tree(&path);
    }

    Ok(())
}

fn remove_tree(path: &str) -> Result<(), Error> {
    let dir = match Dir::open(&Path::new(path)) {
        Ok(dir) => dir,
        Err(_) => {
            return Dir::remove(&Path::new(path));
        },
    };

    let mut children = alloc::vec::Vec::new();
    for child in dir.list()? {
        children.push(child.name);
    }
    drop(dir);

    for child in children {
        let child_path = format!("{}/{}", path, child);
        let _ = remove_tree(&child_path);
    }

    Dir::remove(&Path::new(path))
}

fn process_log_path(root: &str, bytes: &[u8]) -> Option<String> {
    let record = str::from_utf8(bytes).ok()?;
    let pid = json_usize(record, "pid")?;
    let process = json_string(record, "process").unwrap_or_else(|| String::from("unknown"));
    Some(format!("{}/{}-{}.vlog", root, pid, sanitize_name(&process)))
}

fn json_usize(record: &str, key: &str) -> Option<usize> {
    let marker = format!("\"{}\":", key);
    let start = record.find(&marker)? + marker.len();
    let bytes = record.as_bytes();
    let mut end = start;

    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }

    record[start..end].parse().ok()
}

fn json_string(record: &str, key: &str) -> Option<String> {
    let marker = format!("\"{}\":\"", key);
    let mut pos = record.find(&marker)? + marker.len();
    let bytes = record.as_bytes();
    let mut value = String::new();

    while pos < bytes.len() {
        match bytes[pos] {
            b'"' => return Some(value),
            b'\\' => {
                pos += 1;
                if pos >= bytes.len() {
                    return None;
                }

                match bytes[pos] {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'n' => value.push('\n'),
                    b'r' => value.push('\r'),
                    b't' => value.push('\t'),
                    other => value.push(other as char),
                }
            },
            byte => value.push(byte as char),
        }

        pos += 1;
    }

    None
}

fn sanitize_name(name: &str) -> String {
    let mut out = String::new();

    for ch in name.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => out.push(ch),
            _ => out.push('_'),
        }
    }

    if out.is_empty() {
        out.push_str("unknown");
    }
    out 
}
