use super::*;
use core::fmt::Debug;

#[repr(C)]
#[derive(Debug)]
pub enum ChannelOp {
    PushSmall { data: [u8; 64], len: u8 }, 
    PushLarge { vmo_handle: HandleID, offset: usize, len: usize },
    Pull { buffer_ptr: *mut u8 },
}

#[repr(C)]
#[derive(Debug)]
pub enum SocketOp {
    Create { sourceproc: HandleID, sinkproc: HandleID },
    SetNB { nb: bool },   // non blocking not non binary. but could be non binary. up to u.
}

#[repr(C)]
#[derive(Debug)]
pub enum DirectoryOp {
    Link { name: *const u8, name_len: usize, handle_id: HandleID },
    Unlink { name: *const u8, name_len: usize },
    Lookup { name: *const u8, name_len: usize },
    List { offset: usize, sink: HandleID },
}

#[repr(C)]
#[derive(Debug)]
pub enum FileOp {
    Read { offset: usize, buffer_ptr: *mut u8, len: usize },
    Write { offset: usize, buffer_ptr: *mut u8, len: usize },
    Stat,
}

#[repr(C)]
#[derive(Debug)]
pub enum VmoOp {
    GetPage { offset: usize },
    Resize { new_size: usize },
    Clone { offset: usize, len: usize },
    MapIntoProc { vaddr: usize, len: usize, vm_flags: usize }, 
}

#[repr(C)]
#[derive(Debug)]
pub enum ProcOp {
    Kill,
    GetStatus { status_ptr: *mut ProcStatus },
    Unmap { vaddr: usize, len: usize },
    SpawnThread { entry: usize, stack_top: usize, arg: usize, priority: u8 },
}

#[repr(C)]
#[derive(Debug)]
pub enum ThreadOp {
    Kill,
    Join,
    GetID,
}

#[repr(C)]
#[derive(Debug)]
pub enum ProcManOp {
    Spawn { 
        exec_handle: HandleID, 
        root_handle: HandleID, 
        root_rights: AccessRights,
        source: HandleID,
        sink: HandleID,

        extra_handles_ptr: *const crate::HandleGrant,
        extra_handles_len: usize,

        args_buffer_ptr: *const u8,
        args_buffer_len: usize,
    },
}

#[repr(C)]
#[derive(Debug)]
pub enum MemManOp {
    CreatePool { limit: usize },
}

#[repr(C)]
#[derive(Debug)]
pub enum MemPoolOp {
    AllocateVmo { size: usize },
    CreateSubPool { limit: usize },
}

#[repr(C)]
#[derive(Debug)]
pub enum ClockOp {
    GetTimestamp,
}

