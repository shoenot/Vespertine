#![no_std]
#![no_main]

extern crate alloc;

use core::ptr::null;

use alloc::str;
use alloc::string::String;
use alloc::vec::Vec;
use vespertine_abi::AccessRights;
use vespertine_abi::FileOp;
use vespertine_abi::HandleGrant;
use vespertine_abi::HandleID;
use vespertine_abi::Invocation;
use vespertine_abi::ProcManOp;
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::tag::TAG_SYS_PROCMAN;
use vespertine_abi::tag::TAG_SYS_SOCKFAC;
use vespertine_rt::print;
use vespertine_rt::println;
use vespertine_rt::source::read_line;
use vespertine_rt::syscall::sys_close;
use vespertine_rt::syscall::sys_invoke;
use vespertine_rt::syscall::sys_lookup;
use vespertine_std::Error;
use vespertine_std::ErrorKind;
use vespertine_std::Exec;
use vespertine_std::Read;
use vespertine_std::env::root;
use vespertine_std::fs::Dir;
use vespertine_std::fs::DirEntry;
use vespertine_std::fs::File;
use vespertine_std::fs::walk_path;
use vespertine_std::env;
use vespertine_std::socket::Socket;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] shell error: {:?}", e);
    }
}

#[unsafe(no_mangle)]
fn run(pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let pm_handle = env::find_tag(TAG_SYS_PROCMAN)
        .ok_or(Error { kind: ErrorKind::AccessDenied, message: "Process Manager capability not found" })?.id;
    let sf_handle = env::find_tag(TAG_SYS_SOCKFAC) 
        .ok_or(Error { kind: ErrorKind::AccessDenied, message: "Socket Factory capability not found" })?.id;

    loop {
        print!(">> ");
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
            "" => {},
            "echo" => cmd_echo(args_vec),
            "ns" => {
                let mut sock = Socket::new().expect("Error creating socket pair");

                let pmg = HandleGrant { id: pm_handle, rights: AccessRights::all(), tag: TAG_SYS_PROCMAN, };
                let sfg = HandleGrant { id: sf_handle, rights: AccessRights::all(), tag: TAG_SYS_SOCKFAC, };

                let _ = Exec::new("ns".into())
                    .args(&args_vec)
                    .sink(sock.write_handle()?)
                    .root_rights(AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE)
                    .grant(pmg)
                    .grant(sfg)
                    .spawn();

                sock.close_write();
                print_stream(&sock)?;
            }
            other => {println!("unknown command: {}", other)},
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
        let op = Invocation::File(
            FileOp::Read { offset: 0, buffer_ptr: buf.as_mut_ptr(), len: buf.len() }
        );
        match sys_invoke(source, &op) {
            Ok(0) | Err(_) => break,        // EOF or Error
            Ok(n) => {
                let op = Invocation::File(
                    FileOp::Write { offset: 0, buffer_ptr: buf.as_mut_ptr(), len: n }
                );
                let _ = sys_invoke(sink, &op);
            }
        }
    }
}

