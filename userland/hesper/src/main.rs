#![no_std]
#![no_main]

use core::{ptr::null, slice};

use vespertine_abi::{AccessRights, FileOp, HandleGrant, HandleID, Invocation, ProcManOp, ProcessInitPackage, Signal, protocol::{MemoryRequest, ResourceResponse}, tag::*};
use vespertine_rt::{println, syscall::{sys_close, sys_create_socket, sys_invoke, sys_lookup}};
use vespertine_std::{Error, ErrorKind, Exec, Read, Write, env, socket::Socket};

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] Hesper error: {:?}", e);
    }
    let _ = sys_close(pkg.sink_handle);
}

fn run(pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    println!("[INFO] Hesper init system online");

    let pm_handle = env::find_tag(TAG_SYS_PROCMAN)
        .ok_or(Error { kind: ErrorKind::AccessDenied, message: "Process Manager capability not found" })?.id;
    let sf_handle = env::find_tag(TAG_SYS_SOCKFAC) 
        .ok_or(Error { kind: ErrorKind::AccessDenied, message: "Socket Factory capability not found" })?.id;

    // create comms sockets 
    let (hesper_end, client_end) = sys_create_socket(sf_handle).map_err(Error::from)?;
    let hesper_sock = Socket::from_read_handle(hesper_end);
    hesper_sock.setnb(true);

    println!("[INFO] Launching shell...");
    let pm_grant = HandleGrant { id: pm_handle, rights: AccessRights::all(), tag: TAG_SYS_PROCMAN, };
    let sf_grant = HandleGrant { id: sf_handle, rights: AccessRights::all(), tag: TAG_SYS_SOCKFAC, };
    let sock_grant = HandleGrant { id: client_end, rights: AccessRights::READ | AccessRights::WRITE, tag: TAG_SYS_RES_MAN, };

    Exec::new("shell".into())
        .root_rights(AccessRights::all())
        .grant(pm_grant)
        .grant(sf_grant)
        .grant(sock_grant)
        .spawn();
    
    println!("[INFO] Hesper entering event loop...");
    loop { 
        // sleep-wait until socket is readable or peer disconnects 
        hesper_sock.wait(Signal::READABLE | Signal::PEER_CLOSED)?;

        let mut req = MemoryRequest { requested_bytes: 0, pool_handle: HandleID(0) };
        let req_ptr = &mut req as *mut _ as *mut u8;
        let req_size = size_of::<MemoryRequest>();

        let request = unsafe { slice::from_raw_parts_mut(req_ptr, req_size) };
        match hesper_sock.read(request) {
            Ok(n) if n == req_size => {
                println!("[INFO] Hesper allocating {} bytes for pool {:?}", req.requested_bytes, req.pool_handle);
                let resp = ResourceResponse { status: 0 };
                let resp_ptr = &resp as *const _ as *const u8;
                let resp_size = size_of::<ResourceResponse>();

                let response = unsafe { slice::from_raw_parts(resp_ptr, resp_size) };
                let _ = hesper_sock.write(response)?;
            },
            Ok(0) => {
                println!("[INFO] Hesper client socket disconnected");
                break;
            },
            Err(e) if e.kind == ErrorKind::WouldBlock => {
                continue;
            }
            Err(e) => {
                return Err(e);
            },
            _ => continue,
        }
    }
    Ok(())
}
