pub mod futex;

use alloc::string::String;
use alloc::vec::Vec;
use core::mem::zeroed;
use core::sync::atomic::Ordering;

use hal::context::SyscallFrame;
use hal::interrupts::disable_interrupts;
use hal::usercopy::{
    KERNEL_BASE,
    safe_copy_from,
    safe_copy_to,
};
use vespertine_abi::Invocation;
use vespertine_abi::SysError;

use crate::executor::syscall_bridge::{block_on, handle_sys_invoke};
use crate::cpu::current_core_mut;
use crate::executor::waiter::interruptible_wait;
use crate::object::handle::HandleID;
use crate::object::invoke::InvocationError;
use crate::object::vfs::kernel_close;
use crate::sched::dispatch::run_user_kernel_entry;
use crate::sched::scheduler::ScheduleReason;
use crate::process::current_process;
use crate::terminate_thread;

impl From<InvocationError> for SysError {
    fn from(err: InvocationError) -> Self {
        match err {
            InvocationError::AccessDenied => SysError::AccessDenied,
            InvocationError::InvalidHandle => SysError::InvalidHandle,
            InvocationError::InvalidArgument => SysError::InvalidArgument,
            InvocationError::InvalidPointer => SysError::InvalidPointer,
            InvocationError::UnsupportedOperation => SysError::UnsupportedOperation,
            InvocationError::BufferFull => SysError::BufferFull,
            InvocationError::OutOfMemory => SysError::OutOfMemory,
            InvocationError::WouldBlock => SysError::WouldBlock,
            InvocationError::PoolExhausted => SysError::PoolExhausted,
            InvocationError::NameTooLong => SysError::NameTooLong,
            InvocationError::InvalidEncoding => SysError::InvalidEncoding,
            InvocationError::NotMapped => SysError::NotMapped,
            InvocationError::ThreadSpawnFail => SysError::ThreadSpawnFail,
            InvocationError::PathNotFound | InvocationError::NotFound => SysError::NotFound,
            InvocationError::Io => SysError::Io,
            InvocationError::NoSpace => SysError::NoSpace,
            InvocationError::Terminating => unreachable!("thread termination consumed by syscall bridge"), 
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

unsafe fn dispatch_syscall(frame: *mut SyscallFrame) {
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
            },
            3 => {
                current_core_mut().scheduler.schedule(ScheduleReason::Yield);
                Ok(0)
            },
            4 => {
                let addr = (*frame).rdi;
                let expected = (*frame).rsi as u32;
                let process = current_process().unwrap();
                let tcb = current_core_mut().scheduler.get_current_thread();
                let thread = &*tcb;

                match block_on(interruptible_wait(thread.termination_token(), process.futexes.wait(addr, expected))) {
                    Ok(res) => res.map(|()| 0),
                    Err(term) => Err(term.into()),
                }
            },
            5 => {
                let addr = (*frame).rdi;
                let count = (*frame).rsi;
                let process = current_process().unwrap();

                process.futexes.wake(addr, count).map(|()| 0)
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
            Err(InvocationError::Terminating) => return,
            Err(e) => (*frame).rax = SysError::from(e) as usize,
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn syscall_dispatch(frame: *mut SyscallFrame) {
    run_user_kernel_entry(|| unsafe { dispatch_syscall(frame) });
}
