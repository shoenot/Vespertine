use core::arch::asm;

use vespertine_abi::{DirectoryOp, FileOp, HandleID, Invocation, Signal, SocketOp};

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
    WouldBlock = 26,
    PoolExhausted = 27,
    NameTooLong = 28,
    InvalidEncoding = 29,
    NotMapped = 30,

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
            26 => SysError::WouldBlock,
            27 => SysError::PoolExhausted,
            28 => SysError::NameTooLong,
            29 => SysError::InvalidEncoding,
            30 => SysError::NotMapped,
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

pub fn sys_create_socket(factory: HandleID) -> Result<(HandleID, HandleID), SysError> {
    let packed = sys_invoke(factory, &Invocation::Socket(
            SocketOp::Create { sourceproc: HandleID(1), sinkproc: HandleID(1) }
    ))?;
    Ok((HandleID(packed & 0xFFFF_FFFF), HandleID(packed >> 32)))
}

pub fn sys_lookup(dir: HandleID, name: &str) -> Result<HandleID, SysError> {
    let op = Invocation::Directory(DirectoryOp::Lookup {
        name: name.as_ptr(),
        name_len: name.len(),
    });
    let child_handle = sys_invoke(dir, &op)?;
    Ok(HandleID(child_handle))
}

pub fn sys_read(handle: HandleID, buffer_ptr: *mut u8, len: usize, offset: usize) -> Result<usize, SysError> {
    let op = FileOp::Read { offset, buffer_ptr, len };
    sys_invoke(handle, &Invocation::File(op))
}

pub fn sys_write(handle: HandleID, buffer_ptr: *const u8, len: usize, offset: usize) -> Result<usize, SysError> {
    let op = FileOp::Write { offset, buffer_ptr: buffer_ptr as *mut u8, len };
    sys_invoke(handle, &Invocation::File(op))
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

pub fn sys_set_nb(handle: HandleID, nb: bool) -> Result<usize, SysError> {
    sys_invoke(handle, &Invocation::Socket(SocketOp::SetNB { nb }))
}

pub fn sys_wait(handle: HandleID, signal: Signal) -> Result<usize, SysError> {
    sys_invoke(handle, &Invocation::Wait(signal))
}
