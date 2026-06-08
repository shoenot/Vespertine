#![no_std]
#![no_main]

use core::slice;

use vespertine_abi::{
    AccessRights, BrokerOp, HandleGrant, HandleID, Invocation, ProcOp, ProcessInitPackage, Signal, protocol::{MemoryRequest, ResourceResponse}, tag::*
};
use vespertine_rt::{
    println,
    syscall::{sys_close, sys_invoke, sys_sleep},
};
use vespertine_std::{Error, ErrorKind, Exec, Read, Write, env::{self, find_tag}, fs::walk_path, log::{self, SystemLog}, socket::Socket};

use vespertine_rt::thread as rt_thread;

#[repr(C)]
struct BrokerRequest {
    pub tag: usize,
}

#[repr(C)]
struct BrokerResponse {
    pub handle: usize,
}

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

    // create comms socket for terminal
    let (hesper_sock, client_sock) = Socket::new_pair()?;
    let _ = hesper_sock.set_nonblocking(true);

    println!("[INFO] Launching terminal...");
    log.write_string("Launching terminal".into())?;

    Exec::new("terminal".into())
        .source(env::source())
        .sink(env::sink())
        .root_rights(AccessRights::all())
        .grant(TAG_SYS_PROCMAN, AccessRights::all())?
        .grant(TAG_SYS_SOCKFAC, AccessRights::all())?
        .grant(TAG_SYS_LOGGER, AccessRights::WRITE)?
        .grant(TAG_SYS_CLOCK, AccessRights::all())?
        .grant_new(client_sock.handle(), TAG_SYS_SOCKFAC, AccessRights::all())?
        .spawn()?;

    let broker_handle = walk_path("/System/Services/ResourceBroker", env::root())?;

    // broker accept thread
    let _ = rt_thread::spawn(move || {
        loop {  
            let res = sys_invoke(broker_handle, &Invocation::Broker(BrokerOp::Accept));
                match res {
                    Ok(packed) => {
                        let client_socket_handle = HandleID(packed & 0xFFFFFFFF);
                        let client_process_handle = HandleID(packed >> 32);

                        // spawn a handler thread for this client
                        let _ = rt_thread::spawn(move || {
                            let _ = handle_client(client_socket_handle, client_process_handle);
                        });
                    },
                    Err(_) => {
                        let clock = walk_path("/System/Services/Clock", env::root()).unwrap_or(HandleID(0));
                        if clock != HandleID(0) {
                            let _ = sys_sleep(100, clock);
                        }
                    },
                }
            }
      });

    println!("[INFO] Hesper entering event loop...");
    loop {
        // sleep-wait until socket is readable or peer disconnects
        hesper_sock.wait(Signal::READABLE | Signal::PEER_CLOSED)?;

        let mut req = MemoryRequest {
            requested_bytes: 0,
            pool_handle: HandleID(0),
        };
        let req_ptr = &mut req as *mut _ as *mut u8;
        let req_size = size_of::<MemoryRequest>();

        let request = unsafe { slice::from_raw_parts_mut(req_ptr, req_size) };
        match hesper_sock.read(request) {
            Ok(n) if n == req_size => {
                println!(
                    "[INFO] Hesper allocating {} bytes for pool {:?}",
                    req.requested_bytes, req.pool_handle
                );
                let resp = ResourceResponse { status: 0 };
                let resp_ptr = &resp as *const _ as *const u8;
                let resp_size = size_of::<ResourceResponse>();

                let response = unsafe { slice::from_raw_parts(resp_ptr, resp_size) };
                let _ = hesper_sock.write(response)?;
            }
            Ok(0) => {
                println!("[INFO] Hesper client socket disconnected");
                break;
            }
            Err(e) if e.kind == ErrorKind::WouldBlock => {
                continue;
            }
            Err(e) => {
                return Err(e);
            }
            _ => continue,
        }
    }
    Ok(())
}

fn handle_client(sock_handle: HandleID, proc_handle: HandleID) -> Result<(), Error> {
    let sock = Socket::from_handle(sock_handle);
    let proc = Socket::from_handle(proc_handle);

    // cache the capabilities to grant
    let pm = env::find_tag(TAG_SYS_PROCMAN).map(|g| g.id).unwrap_or(HandleID(0));
    let sf = env::find_tag(TAG_SYS_SOCKFAC).map(|g| g.id).unwrap_or(HandleID(0));
    let mm = env::find_tag(TAG_SYS_MEMMAN).map(|g| g.id).unwrap_or(HandleID(0));
    let clk = env::find_tag(TAG_SYS_CLOCK).map(|g| g.id).unwrap_or(HandleID(0));

    loop {
        let mut req = BrokerRequest { tag: 0 };
        let req_ptr = &mut req as *mut _ as *mut u8;
        let req_size = size_of::<BrokerRequest>();
        let request_slice = unsafe { slice::from_raw_parts_mut(req_ptr, req_size) };

        match sock.read(request_slice) {
            Ok(n) if n == req_size => {
                let (source_cap, rights) = match req.tag {
                    TAG_SYS_PROCMAN => (pm, AccessRights::all()),
                    TAG_SYS_SOCKFAC => (sf, AccessRights::all()),
                    TAG_SYS_MEMMAN => (mm, AccessRights::all()),
                    TAG_SYS_CLOCK => (clk, AccessRights::READ | AccessRights::WRITE),
                    _ => (HandleID(0), AccessRights::new()),
                };

                let mut resp = BrokerResponse { handle: 0 };

                if source_cap != HandleID(0) {
                    let insert_op = Invocation::Proc(ProcOp::InsertHandle {
                            source_handle: source_cap,
                            rights,
                    });
                    match sys_invoke(proc.handle(), &insert_op) {
                        Ok(new_child_handle) => {
                            resp.handle = new_child_handle;
                        },
                        Err(_) => {
                            resp.handle = 0;
                        }
                    }
                }

                let resp_ptr = &resp as *const _ as *const u8;
                let resp_size = size_of::<BrokerResponse>();
                let response_slice = unsafe { slice::from_raw_parts(resp_ptr, resp_size) };
                let _ = sock.write(response_slice)?;
            },
            Ok(0) => {
                break; // connection closed
            },
            Err(e) if e.kind == ErrorKind::WouldBlock => {
                continue;
            },
            _ => break,
        }
    }

    sock.close();
    proc.close();
    Ok(())
}
