use core::{fmt::Display, intrinsics::copy_nonoverlapping, mem::zeroed};

use alloc::{string::String, vec::Vec};

use crate::{KERNEL_PROCESS, arch::{get_core_data, x86_64::task::context::SyscallFrame}, core::{object::{handle::HandleID, invoke::InvocationError, vfs::{kernel_close, kernel_invoke}}, thread::get_current_process}, klogln, terminate_thread};
use vespertine_abi::Invocation;

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

    pub fn from_invocation_err(err: InvocationError) -> Self {
        match err {
            InvocationError::AccessDenied => SysError::AccessDenied,
            InvocationError::InvalidHandle => SysError::InvalidHandle,
            InvocationError::InvalidArgument => SysError::InvalidArgument,
            InvocationError::UnsupportedOperation => SysError::UnsupportedOperation,
            InvocationError::BufferFull => SysError::BufferFull,
            InvocationError::OutOfMemory => SysError::OutOfMemory,
            InvocationError::PathNotFound => SysError::BadAddress,
        }
    }
}

unsafe extern "sysv64" {
    pub fn copy_from_user(dst: *mut u8, src: *const u8, len: usize) -> bool;
    pub fn copy_to_user(dst: *mut u8, src: *const u8, len: usize) -> bool;
}

pub fn fetch_user_string(ptr: *const u8, len: usize, strlen_max: usize) -> Result<String, SysError> {
    if len > strlen_max { return Err(SysError::InvalidArgument) };
    let mut str_buf = Vec::with_capacity(len);
    let str_buf_ptr = str_buf.as_mut_ptr();

    unsafe {
        safe_copy_from(str_buf_ptr, ptr, len);
        str_buf.set_len(len);
    }

    String::from_utf8(str_buf).map_err(|_| SysError::InvalidArgument)
}

pub fn give_user_string(user_buffer: *mut u8, kernel_string: String) -> Result<(), SysError> {
    let bytes = kernel_string.as_bytes();
    let len = bytes.len();
    let src_ptr = bytes.as_ptr();

    if user_buffer.is_null() { return Err(SysError::BadAddress) };

    safe_copy_to(user_buffer, src_ptr, len);

    Ok(())
}

pub fn safe_copy_from(dst: *mut u8, src: *const u8, len: usize) -> bool {
    let proc = get_current_process().unwrap();
    if alloc::sync::Arc::ptr_eq(proc, KERNEL_PROCESS.get().unwrap()) {
        unsafe { copy_nonoverlapping(src, dst, len); }
        true
    } else {
        unsafe { copy_from_user(dst, src, len) }
    }
}

pub fn safe_copy_to(dst: *mut u8, src: *const u8, len: usize) -> bool {
    let proc = get_current_process().unwrap();
    if alloc::sync::Arc::ptr_eq(proc, KERNEL_PROCESS.get().unwrap()) {
        unsafe { copy_nonoverlapping(src, dst, len); }
        true
    } else {
        unsafe { copy_to_user(dst, src, len) }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn syscall_dispatch(frame: *mut SyscallFrame) {
    unsafe {
        let syscall_number = (*frame).rax;
        let handle_id = (*frame).rdi;
        let uspace_inv_ptr = (*frame).rsi as *const Invocation;

        klogln!("[INFO] *SYSCALL*: number: {:?}, handle_id: {:?}, uspace_inv_ptr: {:?}", syscall_number, handle_id, uspace_inv_ptr);
        let ret = match syscall_number {
            0 => {
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

                kernel_invoke(HandleID(handle_id), kspace_inv)
            },
            1 => {
                match kernel_close(HandleID(handle_id)) {
                    Ok(_) => Ok(0),
                    Err(e) => Err(e),
                }
            }
            2 | 3 => { 
                terminate_thread!();
            },
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
            Err(e) => (*frame).rax = SysError::from_invocation_err(e) as usize,
        }
    }
}
