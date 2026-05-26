#![no_std]
#![no_main]

extern crate alloc;

use alloc::str;
use alloc::vec::Vec;
use vespertine_abi::FileOp;
use vespertine_abi::HandleID;
use vespertine_abi::Invocation;
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::tag::TAG_SYS_PROCMAN;
use vespertine_abi::tag::TAG_SYS_SOCKFAC;
use vespertine_rt::print;
use vespertine_rt::println;
use vespertine_rt::source::read_line;
use vespertine_rt::syscall::sys_invoke;
use vespertine_std::Read;
use vespertine_std::env::root;
use vespertine_std::fs::Dir;
use vespertine_std::fs::DirEntry;
use vespertine_std::fs::File;
use vespertine_std::fs::walk_path;
use vespertine_std::env;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    let pm = env::find_tag(TAG_SYS_PROCMAN).map(|g| g.id);
    let sf = env::find_tag(TAG_SYS_SOCKFAC).map(|g| g.id);

    loop {
        print!(">> ");
        let mut buf = [0u8; 128];
        let n = read_line(&mut buf);
        let line = str::from_utf8(&buf[..n])
            .unwrap_or("")
            .trim_end_matches('\n')
            .trim();

        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();

        match cmd {
            "" => {},
            "ls" => {
                let dir = if arg.is_empty() { 
                    Dir::from(env::root()) 
                } else { 
                    Dir::open(arg).expect("Could not resolve path")
                };
                let dir_iter = dir.list().expect("Failed to read dir");
                let contents: Vec<DirEntry> = dir_iter.collect();
                for entry in contents {
                    println!("{}", entry);
                }
            },
            "cat" => cmd_cat(arg),
            "echo" => cmd_echo(arg),
            other => {println!("unknown command: {}", other)},
        }
    }
}

fn cmd_echo(text: &str) {
    println!("{}", text);
}

pub fn print_stream<R: Read>(stream: &R) {
    match stream.read_to_string() {
        Ok(text) => print!("{}", text),
        Err(_) => println!("Error reading stream"),
    }
}

fn cmd_cat(path: &str) {
    let file = File::open(path).expect("No such file!");
    print_stream(&file);
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

