#![no_std]
#![no_main]

use vespertine_abi::{AccessRights, ProcessInitPackage, tag::CAP_LOGGER};
use vespertine_rt::{println, syscall::sys_close};
use vespertine_std::{Error, Exec, Write, env, log::SystemLog};

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] Hesper error: {:?}", e);
    }
    let _ = sys_close(pkg.sink_handle);
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let log = SystemLog::connect();
    println!("[INFO] Hesper init system online");
    log.write_string("Hesper init system online".into())?;

    println!("[INFO] Launching terminal...");
    log.write_string("Launching terminal".into())?;

    Exec::new("terminal".into())
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .root_rights(AccessRights::all())
        .grant(CAP_LOGGER, AccessRights::WRITE)?
        .spawn()?;

    println!("[INFO] Hesper initialization complete");
    Ok(())
}
