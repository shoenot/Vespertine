#![no_std]
#![no_main]
extern crate alloc;
use alloc::format;
use alloc::string::String;
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::tag::*;
use vespertine_rt::print;
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_rt::syscall::sys_sleep;
use vespertine_std::Error;
use vespertine_std::ErrorKind;
use vespertine_std::Read;
use vespertine_std::Write;
use vespertine_std::env;
use vespertine_std::fs::Dir;
use vespertine_std::fs::File;
use vespertine_std::fs::walk_path;
use vespertine_rt::thread as rt_thread;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] ns error: {:?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
