#![no_std]
#![no_main]

use vespertine_abi::{AccessRights, HandleID, Invocation, ProcManOp};
use vespertine_rt::syscall::{sys_invoke, sys_lookup};

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(root: HandleID, self_hd: HandleID, source: HandleID, sink: HandleID) {
    // userspace shell proc
    let programs_dir_handle = sys_lookup(root, "Programs").expect("No programs dir");
    let shell_exec_handle = sys_lookup(programs_dir_handle, "shell").expect("No shell executable");
    let root_handle = HandleID(0);
    let root_rights = AccessRights::all();
    let shell_spawn_op = ProcManOp::Spawn { 
        exec_handle: shell_exec_handle, root_handle, root_rights, source, sink 
    };

    sys_invoke(self_hd, &Invocation::ProcessManager(shell_spawn_op))
        .expect("Failed to spawn shell from hesper");
}
