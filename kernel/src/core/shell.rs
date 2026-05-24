use core::str::from_utf8;

use crate::{core::object::{handle::HandleID, invoke::Invocation, op::ChannelOp, vfs::{debug_dump_handles, kernel_invoke}}, klogln};

pub extern "C" fn kernel_shell_thread(chan_handle_id: usize) -> ! {
    let chan_handle = HandleID(chan_handle_id);
    let mut command_bytes = [0u8; 128];

    loop {
        let pull_op = Invocation::Channel(ChannelOp::Pull { 
            buffer_ptr: command_bytes.as_mut_ptr(),
        });

        if let Ok(len) = kernel_invoke(chan_handle, pull_op) {
            if len > 0 {
                if let Ok(cmd_str) = from_utf8(&command_bytes[0..len]) {
                    let ret = execute_shell_cmd(cmd_str);
                    klogln!("{}", ret);
                }
            }

            let ack_op = Invocation::Channel(ChannelOp::PushSmall { data: [0u8; 64], len: 0 });
            let _ = kernel_invoke(chan_handle, ack_op);
        }
    }

}

fn execute_shell_cmd(cmd: &str) -> &str {
    if cmd == "tree" {
        debug_dump_handles();
        ""
    } else { 
        "" 
    }
}
