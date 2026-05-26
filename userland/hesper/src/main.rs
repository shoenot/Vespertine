#![no_std]
#![no_main]

use core::ptr::null;

use vespertine_abi::{AccessRights, FileOp, HandleGrant, Invocation, ProcManOp, ProcessInitPackage, tag::*};
use vespertine_rt::{println, syscall::{sys_invoke, sys_lookup}};
use vespertine_std::env::find_tag;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    // userspace shell proc
    let pm_handle = match find_tag(TAG_SYS_PROCMAN) {
        Some(g) => g,
        None => panic!("Hesper reqires the ProcessManager handle to be injected"),
    }.id;

    let sf_handle = match find_tag(TAG_SYS_SOCKFAC) {
        Some(g) => g,
        None => panic!("Hesper reqires the SocketFactory handle to be injected"),
    }.id;

    let programs_dir_handle = sys_lookup(pkg.root_handle, "Programs").expect("No programs dir");
    let shell_exec_handle = sys_lookup(programs_dir_handle, "shell").expect("No shell executable");

    println!("[INFO] Hesper init system online");
    println!("[INFO] Launching shell...");
    let extra_handles = [
        HandleGrant { id: pm_handle, rights: AccessRights::WRITE, tag: TAG_SYS_PROCMAN, },
        HandleGrant { id: sf_handle, rights: AccessRights::all(), tag: TAG_SYS_SOCKFAC, },
    ]; 

    let shell_spawn_op = ProcManOp::Spawn { 
        exec_handle: shell_exec_handle, 
        root_handle: pkg.root_handle, 
        root_rights: AccessRights::all(), 
        source: pkg.source_handle,
        sink: pkg.sink_handle,
        extra_handles_ptr: extra_handles.as_ptr(),
        extra_handles_len: extra_handles.len(),
        args_buffer_ptr: null(),
        args_buffer_len: 0,
    };

    sys_invoke(pm_handle, &Invocation::ProcessManager(shell_spawn_op))
        .expect("Failed to spawn shell from hesper");
}
