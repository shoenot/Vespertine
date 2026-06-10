#![no_std]
#![no_main]

extern crate alloc;

mod launch;

use alloc::str;
use alloc::string::String;
use alloc::vec::Vec;
use vespertine_abi::AccessRights;
use vespertine_abi::FileOp;
use vespertine_abi::HandleGrant;
use vespertine_abi::HandleID;
use vespertine_abi::Invocation;
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::app::termios::TermCommand;
use vespertine_abi::protocol::PacketFlags;
use vespertine_abi::protocol::PacketHeader;
use vespertine_abi::protocol::PacketType;
use vespertine_abi::protocol::VESPER_MAGIC;
use vespertine_abi::tag::TAG_APP_TERM;
use vespertine_abi::tag::TAG_SYS_CLOCK;
use vespertine_abi::tag::TAG_SYS_PROCMAN;
use vespertine_abi::tag::TAG_SYS_SOCKFAC;
use vespertine_rt::print;
use vespertine_rt::println;
use vespertine_rt::source::read_line;
use vespertine_rt::syscall::sys_invoke;
use vespertine_rt::syscall::sys_write_bytes;
use vespertine_std::Error;
use vespertine_std::ErrorKind;
use vespertine_std::Exec;
use vespertine_std::Read;
use vespertine_std::env;
use vespertine_std::fs::walk_path;
use vespertine_std::socket::Socket;
use vespertine_std::term::unset_raw_mode;

use crate::launch::launch_command;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] shell error: {:?}", e);
    }
}

#[unsafe(no_mangle)]
fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let pm_handle = env::find_tag(TAG_SYS_PROCMAN)
        .ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "Process Manager capability not found".into(),
        })?
        .id;
    let sf_handle = env::find_tag(TAG_SYS_SOCKFAC)
        .ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "Socket Factory capability not found".into(),
        })?
        .id;
    let ctrl_handle = env::find_tag(TAG_APP_TERM)
        .ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "Term control capability not found".into(),
        })?
        .id;

    let ctrl_sock = Socket::from_handle(ctrl_handle);

    loop {
        let mut kbd_backlog: Vec<u8> = Vec::new();
        if let Some(col) = get_cursor_column(&mut kbd_backlog) {
            if col > 1 {
                // print newline if the last program's output didn't do it
                println!("");
            }
        }

        print!("\x1b[35m>> \x1b[0m");
        let mut buf = [0u8; 128];
        let n = read_line(&mut buf);
        let line = str::from_utf8(&buf[..n])
            .unwrap_or("")
            .trim_end_matches('\n')
            .trim();

        let mut words = line.split_whitespace();

        let cmd = words.next().unwrap_or("");

        let args_vec: Vec<String> = words.map(|s| s.into()).collect();

        match cmd {
            "" => {}
            "echo" => cmd_echo(args_vec),
            other => launch_command(other, &args_vec),
        }
    }
}

fn cmd_echo(args: Vec<String>) {
    for arg in args {
        println!("{}", arg);
    }
}

pub fn print_stream<R: Read>(stream: &R) -> Result<(), Error> {
    let text = stream.read_to_string()?;
    print!("{}", text);
    Ok(())
}

pub fn pipe_to_sink(source: HandleID, sink: HandleID) {
    let mut buf = [0u8; 128];
    loop {
        let op = Invocation::File(FileOp::Read {
            offset: 0,
            buffer_ptr: buf.as_mut_ptr() as usize,
            len: buf.len(),
        });
        match sys_invoke(source, &op) {
            Ok(0) | Err(_) => break, // EOF or Error
            Ok(n) => {
                let op = Invocation::File(FileOp::Write {
                    offset: 0,
                    buffer_ptr: buf.as_mut_ptr() as usize,
                    len: n,
                });
                let _ = sys_invoke(sink, &op);
            }
        }
    }
}

fn read_char(kbd_backlog: &mut Vec<u8>) -> Option<u8> {
    if !kbd_backlog.is_empty() {
        return Some(kbd_backlog.remove(0));
    }
    let mut b = 0u8;
    let op = FileOp::Read {
        offset: 0,
        buffer_ptr: &mut b as *mut u8 as usize,
        len: 1,
    };
    match sys_invoke(env::source(), &Invocation::File(op)) {
        Ok(n) if n > 0 => Some(b),
        _ => None,
    }
}

#[allow(unused_assignments)]
fn get_cursor_column(kbd_backlog: &mut Vec<u8>) -> Option<usize> {
    // send the ANSI esc sequence query
    vespertine_rt::print!("\x1b[6n");

    // expect escape (0x1B)
    let esc = read_char(kbd_backlog)?;
    if esc != 0x1B {
        // not escape, so push it to backlog
        kbd_backlog.push(esc);
        return None;
    }

    // expect '[' (0x5B)
    let bracket = read_char(kbd_backlog)?;
    if bracket != 0x5B {
        kbd_backlog.push(esc);
        kbd_backlog.push(bracket);
        return None;
    }

    // read row digits until semicolon
    let mut row_bytes = Vec::new();
    let mut found_semicolon = false;
    loop {
        let c = read_char(kbd_backlog)?;
        if c == b';' {
            found_semicolon = true;
            break;
        }
        if c.is_ascii_digit() {
            row_bytes.push(c);
        } else {
            // put everything back in the backlog if shit doesn't come thru right
            kbd_backlog.push(esc);
            kbd_backlog.push(bracket);
            kbd_backlog.extend(row_bytes);
            kbd_backlog.push(c);
            return None;
        }
    }

    if !found_semicolon {
        return None;
    }

    // read col digits until R
    let mut col = 0;
    let mut col_bytes = Vec::new();
    let mut found_r = false;
    loop {
        let c = read_char(kbd_backlog)?;
        if c == b'R' {
            found_r = true;
            break;
        }
        if c.is_ascii_digit() {
            col = col * 10 + (c - b'0') as usize;
            col_bytes.push(c);
        } else {
            // you know the drill
            kbd_backlog.push(esc);
            kbd_backlog.push(bracket);
            kbd_backlog.extend(row_bytes);
            kbd_backlog.push(b';');
            kbd_backlog.extend(col_bytes);
            kbd_backlog.push(c);
            return None;
        }
    }

    if !found_r {
        return None;
    }

    Some(col)
}
