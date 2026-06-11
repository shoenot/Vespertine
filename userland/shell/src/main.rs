#![no_std]
#![no_main]

extern crate alloc;

mod launch;

use alloc::str;
use alloc::string::String;
use alloc::vec::Vec;
use vespertine_abi::AccessRights;
use vespertine_abi::FileOp;
use vespertine_abi::HandleID;
use vespertine_abi::Invocation;
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::app::termios::TermCommand;
use vespertine_abi::protocol::PacketFlags;
use vespertine_abi::protocol::PacketHeader;
use vespertine_abi::protocol::PacketType;
use vespertine_abi::protocol::VESPER_MAGIC;
use vespertine_abi::tag::CAP_APP_TERMCTRL;
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
    let term_ctrl = env::capability(CAP_APP_TERMCTRL)
        .ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "Shell was not granted terminal control capability".into()
        })?;
    let ctrl_sock = Socket::from_handle(term_ctrl.id);

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
