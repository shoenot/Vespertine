use crate::{arch::x86_64::task::syscall::SysError, core::object::invoke::Invocation, klogln};
use core::arch::asm;

pub fn sys_invoke(handle_id: usize, invocation: Invocation) -> Result<usize, SysError> {
    let inv_ptr = &invocation as *const Invocation as usize;
    let status: usize;
    let payload: usize;
    klogln!("point 4");

    unsafe {
        asm!(
            "syscall",
            in("rax") 0, in("rdi") handle_id, in("rsi") inv_ptr,
            lateout("rax") status, lateout("rdx") payload, out("rcx") _, out("r11") _,
            options(nostack)
        );
    }

    match status {
        0 => Ok(payload),
        _ => Err(SysError::from(status)),
    }
}

