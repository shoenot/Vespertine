use core::{fmt::Display, mem::zeroed};

use crate::{arch::x86_64::task::context::SyscallFrame, kernel::object::{handle::HandleID, invoke::Invocation, vfs::kernel_invoke}};

pub enum SysError {
    Success = 0,

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

impl Display for SysError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SysError::Success => write!(f, "SYSCALL SUCCESS"),

            SysError::InvalidPointer => write!(f, "SYSCALL ERROR: Invalid pointer"),
            SysError::BadAddress => write!(f, "SYSCALL ERROR: Bad address"),
            SysError::OutOfMemory => write!(f, "SYSCALL ERROR: Out of memory"),

            SysError::InvalidHandle => write!(f, "SYSCALL ERROR: Invalid handle"),
            SysError::AccessDenied => write!(f, "SYSCALL ERROR: Access denied"),
            SysError::InvalidArgument => write!(f, "SYSCALL ERROR: Invalid argument"),
            SysError::UnsupportedOperation => write!(f, "SYSCALL ERROR: Unsupported operation"),
            SysError::BufferFull => write!(f, "SYSCALL ERROR: Buffer full"),

            SysError::UnknownSyscall => write!(f, "SYSCALL ERROR: Unknown syscall"),

        }
    }
}

impl SysError {
    pub fn from(status: usize) -> Self {
        match status {
            0 => SysError::Success,
            1 => SysError::InvalidPointer,
            2 => SysError::BadAddress,
            3 => SysError::OutOfMemory,
            21 => SysError::InvalidHandle,
            22 => SysError::AccessDenied,
            23 => SysError::InvalidArgument,
            24 => SysError::UnsupportedOperation,
            25 => SysError::BufferFull,
            41 => SysError::UnknownSyscall,
            _ => SysError::UnknownSyscall,
        }
    }
}

unsafe extern "sysv64" {
    fn copy_from_user(dst: *mut u8, src: *const u8, len: usize) -> bool;
}

#[unsafe(no_mangle)]
pub extern "C" fn syscall_dispatch(frame: *mut SyscallFrame) {
    unsafe {
        let syscall_number = (*frame).rax;
        let handle_id = (*frame).rdi;
        let uspace_inv_ptr = (*frame).rsi as *const Invocation;

        if uspace_inv_ptr as usize >= 0xFFFF_8000_0000_0000 {
            (*frame).rax = SysError::InvalidPointer as usize;
            return;
        }

        // copy from uspace to kspace
        let mut kspace_inv = zeroed::<Invocation>();
        let copy_success = copy_from_user(
            &mut kspace_inv as *mut _ as *mut u8,
            uspace_inv_ptr as *const u8,
            size_of::<Invocation>()
        );

        if !copy_success {
            (*frame).rax = SysError::BadAddress as usize;
            return;
        }

        let ret = match syscall_number {
            0 => kernel_invoke(HandleID(handle_id), kspace_inv),
            1 => todo!(),
            _ => {
                (*frame).rax = SysError::UnknownSyscall as usize;
                return;
            },
        };

        match ret {
            Ok(payload) => {
                (*frame).rax = SysError::Success as usize;
                (*frame).rdx = payload;
            },
            Err(e) => (*frame).rax = e as usize,
        }
    }
}
