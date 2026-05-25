#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::string::ToString;
use vespertine_abi::FileOp;
use vespertine_abi::HandleID;
use vespertine_abi::Invocation;
use vespertine_rt::syscall::sys_invoke;

fn console_write(text: &str) -> Invocation {
    Invocation::File(FileOp::Write { 
        offset: 0, 
        buffer_ptr: text.as_ptr() as *mut u8,
        len: text.len() 
    })
}

fn console_write_static(text: &str) -> Invocation {
   Invocation::File(FileOp::Write { 
        offset: 0, 
        buffer_ptr: text.as_ptr() as *mut u8,
        len: text.len() 
    })
}

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(_root: HandleID, _self: HandleID, _source: HandleID, console: HandleID) {
    let text = "Hello from userland program 1".to_string();
    let op = console_write(&text);
    let _ = sys_invoke(console, &op);
}
