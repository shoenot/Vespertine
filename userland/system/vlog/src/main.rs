#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use vespertine_abi::{Invocation, LogLevel, ProcInfo};
use vespertine_abi::tag::{CAP_LOGGER, CAP_SERIAL_WRITE};
use vstd::broker::Broker;
use vstd::portal::PortalFactory;
use vstd::proc::Waiter;
use vstd::Write;
use core::str;
use core::fmt::Write as _;

use vespertine_common::datetime::epoch_to_datetime;
use vrt::syscall::sys_close;
use vstd::clock::Time;
use vstd::fs::{link_object, resolve};
use vstd::log::MAX_LOG_MESSAGE_BYTES;
use vstd::{Process, prelude::*};

const LOG_ROOT: &str = "/System/Logs";
const LOG_BOOT_ROOT: &str = "/System/Logs/Boots";
const MAX_RECORD_BYTES: usize = 4096;
const RETAIN_BOOT_COUNT: usize = 8;

// 4 for msg len and 1 for log level
const HEADER_BYTES: usize = 5;

fn publish_log() -> Result<Socket, Error> {
    let services = resolve(&Path::new("/System/Services"), AccessRights::CREATE)?;
    let res = (|| {
        let factory = PortalFactory::request()?;
        let (portal, accept) = factory.create(CAP_LOGGER, AccessRights::WRITE)?;
        let accept = Socket::from_handle(accept);

        let linked = link_object(services, "Log", portal);
        let _ = sys_close(portal);
        linked?;
        Ok(accept)
    })();

    let _ = sys_close(services);
    res
}

struct Client {
    socket: Socket,
    caller: ProcInfo,
    buffer: [u8; HEADER_BYTES + MAX_LOG_MESSAGE_BYTES],
    filled: usize,
    target: usize,
}

impl Client {
    // returns the end of one complete frame or None if more bytes are needed
    fn read_message(&mut self) -> Result<Option<(usize, LogLevel)>, Error> {
        loop {
            match self.socket.read(&mut self.buffer[self.filled..self.target]) {
                Ok(0) => return Err(Error::end_of_stream("Log client closed".into())),
                Ok(count) => self.filled += count,
                Err(error) if error.kind == ErrorKind::WouldBlock => return Ok(None),
                Err(error) => return Err(error),
            }

            if self.filled < self.target {
                continue;
            }

            if self.target == HEADER_BYTES {
                let header = [self.buffer[0], self.buffer[1], self.buffer[2], self.buffer[3]];
                let len = u32::from_le_bytes(header) as usize;
                if len == 0 || len > MAX_LOG_MESSAGE_BYTES {
                    return Err(Error::invalid_argument("Invalid log message length".into()));
                }
                self.target = HEADER_BYTES + len;
                continue;
            }

            let end = self.target;
            let log_level = LogLevel::from_raw(self.buffer[4]);
            self.filled = 0;
            self.target = HEADER_BYTES;
            return Ok(Some((end, log_level)));
        }
    }
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

fn open_serial_sink() -> Result<Socket, Error> {
    let serial_grant = env::capability(CAP_SERIAL_WRITE)
        .ok_or_else(|| Error::access_denied("vlog requires a serial opener grant".into()))?;

    let serial_broker = Broker::from_handle(serial_grant.id);
    let serial_socket_id = serial_broker.request(CAP_SERIAL_WRITE, AccessRights::WRITE)?;
    Ok(Socket::from_handle(serial_socket_id))
}

#[vapp::main]
fn main() -> Result<(), Error> {
    println!("vlog starting");

    ensure_dir(LOG_ROOT)?;
    ensure_dir(LOG_BOOT_ROOT)?;

    let boot_id = boot_id();
    let boot_dir = format!("{}/{}", LOG_BOOT_ROOT, boot_id);
    let process_dir = format!("{}/processes", boot_dir);

    ensure_dir(&boot_dir)?;
    ensure_dir(&process_dir)?;
    prune_old_boots()?;

    let mut index = AppendFile::open(&format!("{}/index.vlog", boot_dir))?;
    let mut processes = BTreeMap::<String, AppendFile>::new();
    let mut buf = [0u8; MAX_RECORD_BYTES];

    println!("vlog online: {}", boot_id);

    let mut serial = match open_serial_sink() {
        Ok(sink) => Some(sink),
        Err(e) => {
            println!("[WARN] [vlog] Could not open serial device: {:?}", e);
            None
        }
    };

    let accept = publish_log()?;
    let mut clients = Vec::<Client>::new();

    loop {
        let mut waiter = Waiter::new().readable(accept.handle());
        for client in &clients {
            waiter = waiter.readable(client.socket.handle());
        }
        waiter.wait()?;

        for i in (0..clients.len()).rev() {
            if !waiter.ready(i + 1) {
                continue;
            }

            match clients[i].read_message() {
                Ok(Some((end, level))) => {
                    let client = &clients[i];
                    let message = &client.buffer[HEADER_BYTES..end];
                    let record = build_record(&client.caller, message);
                    let levelstr = level.as_str();

                    index.append(record.as_bytes())?;

                    let path = format!(
                        "{}/{}-{}.vlog",
                        process_dir,
                        client.caller.pid,
                        sanitize_name(client.caller.name()),
                    );
                    if !processes.contains_key(&path) {
                        let file = AppendFile::open(&path)?;
                        processes.insert(path.clone(), file);
                    }
                    if let Some(file) = processes.get_mut(&path) {
                        file.append(record.as_bytes())?;
                    }

                    if let Some(sink) = serial.as_ref() {
                        let name = client.caller.name().escape_debug();
                        let pid = client.caller.pid;
                        let message = String::from_utf8_lossy(message);
                        let message = message.trim_end_matches(['\r', '\n']).escape_debug();
                        let line = format!("[{levelstr}] [{name} (PID: {pid})] {message}\r\n");

                        if let Err(e) = sink.write_all(line.as_bytes()) {
                            println!("[WARN] [vlog] Serial mirror stopped: {:?}", e);
                            serial = None;
                        }
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    clients.swap_remove(i);
                }
            }
        }

        if waiter.ready(0) {
            let mut bytes = [0u8; 8];
            accept.read_exact(&mut bytes)?;
            let session = HandleID(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize);
            let process = HandleID(u32::from_le_bytes(bytes[4..].try_into().unwrap()) as usize);

            let socket = Socket::from_handle(session);
            let process = Process::from_handle(process);

            if clients.len() < 63
                && let Ok(caller) = process.info()
                && socket.set_nonblocking(true).is_ok()
            {
                clients.push(Client {
                    socket,
                    caller,
                    buffer: [0; HEADER_BYTES + MAX_LOG_MESSAGE_BYTES],
                    filled: 0,
                    target: HEADER_BYTES,
                });
            }
        }
    }
}

fn push_json_string(dst: &mut String, value: &str) {
    let _ = dst.write_char('"');
    push_json_escaped(dst, value.as_bytes());
    let _ = dst.write_char('"');
}

fn push_json_bytes(dst: &mut String, value: &[u8]) {
    let _ = dst.write_char('"');
    push_json_escaped(dst, value);
    let _ = dst.write_char('"');
}

fn push_json_escaped(dst: &mut String, value: &[u8]) {
    for byte in value {
        match *byte {
            b'"' => {
                let _ = dst.write_str("\\\"");
            }
            b'\\' => {
                let _ = dst.write_str("\\\\");
            }
            b'\n' => {
                let _ = dst.write_str("\\n");
            }
            b'\r' => {
                let _ = dst.write_str("\\r");
            }
            b'\t' => {
                let _ = dst.write_str("\\t");
            }
            0x20..=0x7e => {
                let _ = dst.write_char(*byte as char);
            }
            _ => {
                let _ = write!(dst, "\\u{:04x}", *byte);
            }
        }
    }
}

fn build_record(caller: &ProcInfo, message: &[u8]) -> String {
    let (ts_sec, ts_nsec) = Time::now();
    let mut record = format!(
        "{{\"ts_sec\":{ts_sec},\"ts_nsec\":{ts_nsec},\"pid\":{},\"user\":{},\"process\":",
        caller.pid, caller.user.0,
    );
    push_json_string(&mut record, caller.name());
    record.push_str(",\"level\":\"info\",\"dropped\":0,\"message\":");
    push_json_bytes(&mut record, message);
    record.push_str("}\n");
    record
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
        }
        Err(error) if error.kind == ErrorKind::NotFound => Dir::create_dir(&Path::new(path)).map(|_| ()),
        Err(error) => Err(error),
    }
}

fn prune_old_boots() -> Result<(), Error> {
    let dir = Dir::open(&Path::new(LOG_BOOT_ROOT))?;
    let mut boots = alloc::vec::Vec::new();

    let context = |error: Error| error.context(format_args!("listing log directories in '{LOG_BOOT_ROOT}'"));
    for entry in dir.list().map_err(context)? {
        boots.push(entry.map_err(context)?.name);
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
        }
    };

    let mut children = alloc::vec::Vec::new();
    let context = |error: Error| error.context(format_args!("listing children of '{path}'"));
    for child in dir.list().map_err(context)? {
        children.push(child.map_err(context)?.name);
    }
    drop(dir);

    for child in children {
        let child_path = format!("{}/{}", path, child);
        let _ = remove_tree(&child_path);
    }

    Dir::remove(&Path::new(path))
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
