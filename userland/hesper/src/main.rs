#![no_std]
#![no_main]

extern crate alloc;
use alloc::format;
use vespertine_abi::{AccessRights, Invocation, ProcessInitPackage, Signal, WaitItem, WaitOp, tag::{CAP_LAUNCHER_EXEC, CAP_LAUNCHER_GRANT, CAP_LOGGER}};
use vespertine_rt::{println, syscall::{sys_close, sys_invoke}};
use vespertine_std::{Error, Exec, Write, env, log::SystemLog, proc::Waiter, socket::Socket};
use vespertine_std::hesper::{
    recv_app_metadata_request,
    send_app_metadata_response,
};

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

    let (launch_hesp, launch_app) = Socket::new_pair()?;

    println!("[INFO] Launching terminal...");
    log.write_string("Launching terminal".into())?;

    Exec::new("terminal".into())
        .source(env::source())
        .sink(env::sink())
        .cwd(env::cwd(), AccessRights::all())
        .root_rights(AccessRights::all())
        .grant(CAP_LOGGER, AccessRights::WRITE)?
        .grant_new(
            launch_app.handle(),
            CAP_LAUNCHER_EXEC,
            AccessRights::READ | AccessRights::WRITE | AccessRights::EXECUTE,
        )?
        .grant_new(
            env::self_handle(),
            CAP_LAUNCHER_GRANT,
            AccessRights::MUTATE,
        )?
        .spawn()?;

    println!("[INFO] Hesper initialization complete. Entering event loop.");

    let mut waiter = Waiter::new()
        .readable(launch_hesp.handle());

    loop {
        waiter.wait()?;

        if waiter.ready(0) {
            let req = match recv_app_metadata_request(&launch_hesp) {
                Ok(r) => r,
                Err(e) => {
                    let _ = log.write_string("[HESPER] Bad metadata request:".into());
                    let _ = log.write_string(format!("[HESPER] {}", e).into());
                    continue;
                },
            };
        }
    }
}
