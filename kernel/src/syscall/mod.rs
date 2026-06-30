use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Display;
use core::mem::zeroed;

use hal::context::SyscallFrame;
use hal::interrupts::{
    disable_interrupts,
    enable_interrupts,
    interrupts_enabled,
};
use hal::usercopy::{
    KERNEL_BASE,
    safe_copy_from,
    safe_copy_to,
};
use vespertine_abi::Invocation;

use crate::core::executor::syscall_bridge::handle_sys_invoke;
use crate::cpu::current_core_mut;
use crate::core::object::handle::HandleID;
use crate::core::object::invoke::InvocationError;
use crate::core::object::vfs::kernel_close;
use crate::sched::dispatch::wake_thread;
use crate::sched::scheduler::ScheduleReason;
use crate::sched::wait::WaitQueue;
use crate::sched::{
    ThreadBlockState,
    ThreadState,
};
use crate::process::current_process;
use crate::terminate_thread;

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
    WouldBlock = 26,
    PoolExhausted = 27,
    NameTooLong = 28,
    InvalidEncoding = 29,
    NotMapped = 30,

    // System Errors
    UnknownSyscall = 41,

    ThreadSpawnFail = 50,
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
            SysError::WouldBlock => write!(f, "SYSCALL ERROR: IO operation would block right now"),
            SysError::PoolExhausted => write!(f, "SYSCALL ERROR: Memory pool exhausted"),
            SysError::NameTooLong => write!(f, "SYSCALL ERROR: Name too long"),
            SysError::InvalidEncoding => write!(f, "SYSCALL ERROR: Invalid encoding"),
            SysError::NotMapped => write!(f, "SYSCALL ERROR: Not mapped"),

            SysError::UnknownSyscall => write!(f, "SYSCALL ERROR: Unknown syscall"),

            SysError::ThreadSpawnFail => write!(f, "SYSCALL ERROR: Thread spawn failed"),
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
            26 => SysError::WouldBlock,
            27 => SysError::PoolExhausted,
            28 => SysError::NameTooLong,
            29 => SysError::InvalidEncoding,
            30 => SysError::NotMapped,
            41 => SysError::UnknownSyscall,
            50 => SysError::ThreadSpawnFail,
            _ => SysError::UnknownSyscall,
        }
    }

    pub fn from_invocation_err(err: InvocationError) -> Self {
        match err {
            InvocationError::AccessDenied => SysError::AccessDenied,
            InvocationError::InvalidHandle => SysError::InvalidHandle,
            InvocationError::InvalidArgument => SysError::InvalidArgument,
            InvocationError::InvalidPointer => SysError::InvalidPointer,
            InvocationError::UnsupportedOperation => SysError::UnsupportedOperation,
            InvocationError::BufferFull => SysError::BufferFull,
            InvocationError::OutOfMemory => SysError::OutOfMemory,
            InvocationError::PathNotFound => SysError::BadAddress,
            InvocationError::WouldBlock => SysError::WouldBlock,
            InvocationError::PoolExhausted => SysError::PoolExhausted,
            InvocationError::NameTooLong => SysError::NameTooLong,
            InvocationError::InvalidEncoding => SysError::InvalidEncoding,
            InvocationError::NotMapped => SysError::NotMapped,
            InvocationError::ThreadSpawnFail => SysError::ThreadSpawnFail,
        }
    }
}

pub fn fetch_user_string(ptr: *const u8, len: usize, strlen_max: usize) -> Result<String, SysError> {
    if len > strlen_max {
        return Err(SysError::InvalidArgument);
    };
    if ptr.is_null() {
        return Err(SysError::BadAddress);
    };

    let end = (ptr as usize).checked_add(len).ok_or(SysError::BadAddress)?;
    if end >= KERNEL_BASE {
        return Err(SysError::BadAddress);
    };

    let mut str_buf = Vec::with_capacity(len);
    let str_buf_ptr = str_buf.as_mut_ptr();

    unsafe {
        if !safe_copy_from(str_buf_ptr, ptr, len) {
            return Err(SysError::BadAddress);
        };
        str_buf.set_len(len);
    }

    String::from_utf8(str_buf).map_err(|_| SysError::InvalidArgument)
}

pub fn give_user_string(user_buffer: *mut u8, kernel_string: String) -> Result<(), SysError> {
    let bytes = kernel_string.as_bytes();
    let len = bytes.len();
    let src_ptr = bytes.as_ptr();

    if user_buffer.is_null() {
        return Err(SysError::BadAddress);
    };

    if !safe_copy_to(user_buffer, src_ptr, len) {
        return Err(SysError::BadAddress);
    };

    Ok(())
}

#[unsafe(no_mangle)]
pub extern "C" fn syscall_dispatch(frame: *mut SyscallFrame) {
    unsafe {
        let syscall_number = (*frame).rax;
        let handle_id = (*frame).rdi;
        let uspace_inv_ptr = (*frame).rsi as *const Invocation;

        // klogln_serial!("[INFO] *SYSCALL*: number: {:?}, handle_id: {:?}, uspace_inv_ptr: {:?}", syscall_number, handle_id, uspace_inv_ptr);
        let ret = match syscall_number {
            0 => {
                if uspace_inv_ptr as usize >= KERNEL_BASE {
                    (*frame).rax = SysError::InvalidPointer as usize;
                    return;
                }

                // copy from uspace to kspace
                let mut kspace_inv = zeroed::<Invocation>();
                if !safe_copy_from(&mut kspace_inv as *mut _ as *mut u8, uspace_inv_ptr as *const u8, size_of::<Invocation>()) {
                    (*frame).rax = SysError::BadAddress as usize;
                    return;
                }

                handle_sys_invoke(HandleID(handle_id), kspace_inv)
            }
            1 => match kernel_close(HandleID(handle_id)) {
                Ok(_) => Ok(0),
                Err(e) => Err(e),
            },
            2 => {
                terminate_thread!((*frame).rdi as u32);
            }
            3 => {
                current_core_mut().scheduler.schedule(ScheduleReason::Yield);
                Ok(0)
            }
            4 => {
                // futex wait (addr, expected)
                let uaddr = (*frame).rdi;
                let expected = (*frame).rsi as u32;
                let proc = current_process().unwrap();

                // check the value
                let mut val = 0u32;
                if safe_copy_from(&mut val as *mut _ as *mut u8, uaddr as *const u8, 4) {
                    if val == expected {
                        let int_state = interrupts_enabled();
                        disable_interrupts();

                        let sched = &mut current_core_mut().scheduler;
                        let mut futexes = proc.futexes.write();

                        let mut current_val = 0u32;
                        if safe_copy_from(&mut current_val as *mut _ as *mut u8, uaddr as *const u8, 4) && current_val == expected {
                            let wq = futexes.entry(uaddr).or_insert_with(WaitQueue::new);
                            let current = sched.get_current_thread();
                            (*current).set_block_state(ThreadBlockState::Futex { addr: uaddr });
                            (*current).transition(ThreadState::Running, ThreadState::Blocked).expect("futex waiter was not running");
                            wq.push(current);
                            drop(futexes);

                            sched.schedule(ScheduleReason::Blocked);
                        } else {
                            drop(futexes);
                        }

                        if int_state {
                            enable_interrupts();
                        }
                    }
                    Ok(0)
                } else {
                    Err(InvocationError::InvalidPointer)
                }
            }
            5 => {
                let uaddr = (*frame).rdi;
                let count = (*frame).rsi;
                let proc = current_process().unwrap();

                let mut futexes = proc.futexes.write();
                if let Some(wq) = futexes.get_mut(&uaddr) {
                    for _ in 0..count {
                        let thread = wq.pop_wakeable();
                        if thread.is_null() {
                            break;
                        }
                        wake_thread(thread);
                    }
                }
                Ok(0)
            }
            _ => {
                (*frame).rax = SysError::UnknownSyscall as usize;
                return;
            }
        };

        match ret {
            Ok(payload) => {
                (*frame).rax = SysError::Success as usize;
                (*frame).rdx = payload;
            }
            Err(e) => (*frame).rax = SysError::from_invocation_err(e) as usize,
        }
    }
}
