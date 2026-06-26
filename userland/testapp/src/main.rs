#![no_std]
#![no_main]

extern crate alloc;
use vespertine_abi::ProcessInitPackage;
use vespertine_rt::{println, syscall::sys_close};
use vespertine_std::{Error, env};

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("testapp error: {:#?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    unsafe {
        let ptr = 0xdeadbeef as *mut u64;
        *ptr = 1337;
    }
    Ok(())
}
