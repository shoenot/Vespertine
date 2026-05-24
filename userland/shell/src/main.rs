#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::string::ToString;
use mnemosyne_abi::FileOp;
use mnemosyne_abi::HandleID;
use mnemosyne_abi::Invocation;
use mnemosyne_rt::syscall::sys_invoke;

fn console_write(mut text: String) -> Invocation {
    let text_ptr = text.as_mut_ptr();
    let text_len = text.len();
    Invocation::File(FileOp::Write { 
        offset: 0, 
        buffer_ptr: text_ptr,
        len: text_len 
    })
}

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(root: HandleID, console:HandleID) {
    let _ = sys_invoke(console, &console_write("Hello from userland program 1".to_string()));
}
