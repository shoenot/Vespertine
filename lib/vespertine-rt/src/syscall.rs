use core::arch::asm;

use vespertine_abi::{
    DirectoryOp, FileOp, HandleID, Invocation, MemPoolOp, ProcOp, Signal, SocketOp, VmoOp, WaitOp,
};

#[derive(Debug)]
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

    // Thread Errors
    ThreadSpawnFail = 50,
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
            50 => SysError::ThreadSpawnFail,
            _ => SysError::UnknownSyscall,
        }
    }
}

//----------------------------------------------------------//
//------------------ RAW SYSTEM CALLS ----------------------//
//----------------------------------------------------------//

/// Primary system call function. Directly calls invocations on the handle provided.
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

/// Top level system call function. Closes the handle.
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
    if ret == 0 {
        Ok(())
    } else {
        Err(SysError::from(ret))
    }
}

//----------------------------------------------------------//
//--------------------- SOCKET HELPERS ---------------------//
//----------------------------------------------------------//

pub fn sys_create_socket(factory: HandleID) -> Result<(HandleID, HandleID), SysError> {
    let packed = sys_invoke(
        factory,
        &Invocation::Socket(SocketOp::Create {
            sourceproc: HandleID(1),
            sinkproc: HandleID(1),
        }),
    )?;
    Ok((HandleID(packed & 0xFFFF_FFFF), HandleID(packed >> 32)))
}

pub fn sys_set_nb(handle: HandleID, nb: bool) -> Result<usize, SysError> {
    sys_invoke(handle, &Invocation::Socket(SocketOp::SetNB { nb }))
}

pub fn sys_wait(handle: HandleID, signal: Signal) -> Result<usize, SysError> {
    sys_invoke(handle, &Invocation::Wait(WaitOp::One(signal)))
}

//----------------------------------------------------------//
//------------------- FILESYSTEM HELPERS -------------------//
//----------------------------------------------------------//

pub fn sys_lookup(dir: HandleID, name: &str) -> Result<HandleID, SysError> {
    let op = Invocation::Directory(DirectoryOp::Lookup {
        name: name.as_ptr() as usize,
        name_len: name.len(),
    });
    let child_handle = sys_invoke(dir, &op)?;
    Ok(HandleID(child_handle))
}

pub fn sys_create_file(dir: HandleID, name: &str) -> Result<HandleID, SysError> {
    let op = Invocation::Directory(DirectoryOp::CreateFile {
        name: name.as_ptr() as usize,
        name_len: name.len(),
    });
    let child = sys_invoke(dir, &op)?;
    Ok(HandleID(child))
}

pub fn sys_create_dir(dir: HandleID, name: &str) -> Result<HandleID, SysError> {
    let op = Invocation::Directory(DirectoryOp::CreateDir {
        name: name.as_ptr() as usize,
        name_len: name.len(),
    });
    let child = sys_invoke(dir, &op)?;
    Ok(HandleID(child))
}

pub fn sys_unlink(dir: HandleID, name: &str) -> Result<(), SysError> {
    let op = Invocation::Directory(DirectoryOp::Unlink {
        name: name.as_ptr() as usize,
        name_len: name.len(),
    });
    sys_invoke(dir, &op)?;
    Ok(())
}

pub fn sys_read(
    handle: HandleID,
    buffer_ptr: *mut u8,
    len: usize,
    offset: usize,
) -> Result<usize, SysError> {
    let op = FileOp::Read {
        offset,
        buffer_ptr: buffer_ptr as usize,
        len,
    };
    sys_invoke(handle, &Invocation::File(op))
}

pub fn sys_write(
    handle: HandleID,
    buffer_ptr: *const u8,
    len: usize,
    offset: usize,
) -> Result<usize, SysError> {
    let op = FileOp::Write {
        offset,
        buffer_ptr: buffer_ptr as *mut u8 as usize,
        len,
    };
    sys_invoke(handle, &Invocation::File(op))
}

pub fn sys_write_bytes(handle: HandleID, data: &[u8]) -> Result<usize, SysError> {
    sys_write(handle, data.as_ptr(), data.len(), 0)
}

//----------------------------------------------------------//
//----------------------- VMO HELPERS ----------------------//
//----------------------------------------------------------//

pub fn sys_mmap(
    mem_pool_handle: HandleID,
    size: usize,
    target_vaddr: usize,
    vm_flags: usize,
) -> Result<usize, SysError> {
    let alloc_op = Invocation::MemPool(MemPoolOp::AllocateVmo { size });
    let vmo_idx = sys_invoke(mem_pool_handle, &alloc_op)?;
    let vmo_handle = HandleID(vmo_idx);

    let map_op = Invocation::Vmo(VmoOp::MapIntoProc {
        vaddr: target_vaddr,
        len: size,
        vm_flags,
    });
    let mapped_addr = sys_invoke(vmo_handle, &map_op);

    let _ = sys_close(vmo_handle);
    mapped_addr
}

pub fn sys_munmap(self_handle: HandleID, vaddr: usize, len: usize) -> Result<(), SysError> {
    let unmap_op = Invocation::Proc(ProcOp::Unmap { vaddr, len });
    sys_invoke(self_handle, &unmap_op)?;
    Ok(())
}

//----------------------------------------------------------//
//--------------------- PROCESS HELPERS --------------------//
//----------------------------------------------------------//

pub fn sys_set_fsbase(self_handle: HandleID, fs_base: usize) -> Result<(), SysError> {
    let op = Invocation::Proc(ProcOp::SetFsBase { fs_base });
    sys_invoke(self_handle, &op)?;
    Ok(())
}
