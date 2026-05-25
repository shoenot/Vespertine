use core::arch::asm;

use vespertine_abi::{DirectoryOp, HandleID, Invocation};

#[derive(Debug)]
pub enum SysError { Success = 0,

    // Memory and pointer Errors
    InvalidPointer = 1,
    BadAddress = 2,
    OutOfMemory = 3,

    // Handle and Capability Errors
    InvalidHandle = 21,
    AccessDenied = 22,
    InvalidArgument = 23,
    UnsupportedOperation = 24,
    BufferFull = 25,

    // System Errors
    UnknownSyscall = 41,
}

impl SysError {
    pub fn from(errnum: usize) -> SysError {
        match errnum {
            0 => SysError::Success,
            1 => SysError::InvalidPointer,
            2 => SysError::BadAddress,
            3 => SysError::OutOfMemory,
            21 => SysError::InvalidHandle,
            22 => SysError::AccessDenied,
            23 => SysError::InvalidArgument,
            24 => SysError::UnsupportedOperation,
            25 => SysError::BufferFull,
            _ => SysError::UnknownSyscall,
        }
    }
}

pub fn sys_invoke(handle: HandleID, op: &Invocation) -> Result<usize, SysError> {
    // rax = 0 (invoke), rdi = HandleID, rsi = Invocation structure.
    let ret: usize;
    let payload: usize;

    unsafe {
        asm!(
            "mov rax, 0",
            "syscall",
            in("rdi") handle.0,
            in("rsi") op as *const Invocation as usize,
            lateout("rax") ret,
            lateout("rdx") payload,
            out("rcx") _, // clobbered
            out("r11") _, // clobbered
        );
    }

    if ret == 0 {
        Ok(payload)
    } else {
        Err(SysError::from(ret))
    }
}

pub fn sys_lookup(dir: HandleID, name: &str) -> Result<HandleID, SysError> {
    let op = Invocation::Directory(DirectoryOp::Lookup {
        name: name.as_ptr(),
        name_len: name.len(),
    });
    let child_handle = sys_invoke(dir, &op)?;
    Ok(HandleID(child_handle))
}

pub fn sys_close(handle: HandleID) -> Result<(), SysError> {
    let ret: usize;
    unsafe {
        asm!(
            "mov rax, 1",
            "syscall",
            in("rdi") handle.0,
            lateout("rax") ret,
            out("rdx") _,   // clobbered
            out("rcx") _,   // clobbered
            out("r11") _,   // clobbered
        );
    }
    if ret == 0 { Ok(()) } else { Err(SysError::from(ret)) }
}
