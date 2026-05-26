#![no_std]
#![no_main]
pub mod op;
pub mod tag;
pub mod protocol;
mod bitwise;

use core::{fmt::Debug, option::Iter, slice};
pub use op::*;

#[repr(C)]
#[derive(Debug)]
pub enum Invocation {
    Ping,
    GetInfo,
    Channel(ChannelOp),
    Directory(DirectoryOp),
    File(FileOp),
    Vmo(VmoOp),
    Proc(ProcOp),
    ProcessManager(ProcManOp),
    MemoryManager(MemManOp),
    MemPool(MemPoolOp),
    Clock(ClockOp),
    Socket(SocketOp),
}

impl Invocation {
    pub fn required_rights(&self) -> AccessRights {
        match self {
            Invocation::Ping => AccessRights::READ,
            Invocation::GetInfo => AccessRights::READ,
            Invocation::Channel(ChannelOp::PushSmall { .. }) => AccessRights::WRITE,
            Invocation::Channel(ChannelOp::PushLarge { .. }) => AccessRights::WRITE,
            Invocation::Channel(ChannelOp::Pull { .. }) => AccessRights::READ,
            Invocation::Directory(DirectoryOp::Link { .. }) => AccessRights::WRITE,
            Invocation::Directory(DirectoryOp::Unlink { .. }) => AccessRights::WRITE,
            Invocation::Directory(DirectoryOp::Lookup { .. }) => AccessRights::READ,
            Invocation::Directory(DirectoryOp::List { .. }) => AccessRights::READ,
            Invocation::File(FileOp::Read { .. }) => AccessRights::READ,
            Invocation::File(FileOp::Write { .. }) => AccessRights::WRITE,
            Invocation::File(FileOp::Stat) => AccessRights::READ,
            Invocation::Vmo(VmoOp::GetPage { .. }) => AccessRights::READ,
            Invocation::Vmo(VmoOp::Resize { .. }) => AccessRights::MUTATE,
            Invocation::Vmo(VmoOp::Clone { .. }) => AccessRights::CREATE,
            Invocation::Vmo(VmoOp::MapIntoProc { .. }) => AccessRights::MUTATE,
            Invocation::Proc(ProcOp::Kill) => AccessRights::WRITE,
            Invocation::Proc(ProcOp::GetStatus { .. }) => AccessRights::READ,
            Invocation::Proc(ProcOp::Unmap { .. }) => AccessRights::MUTATE,
            Invocation::ProcessManager(ProcManOp::Spawn { .. }) => AccessRights::CREATE,
            Invocation::MemoryManager(MemManOp::CreatePool { .. }) => AccessRights::CREATE,
            Invocation::MemPool(MemPoolOp::AllocateVmo { .. }) => AccessRights::CREATE,
            Invocation::MemPool(MemPoolOp::CreateSubPool { .. }) => AccessRights::CREATE,
            Invocation::Clock(ClockOp::GetTimestamp) => AccessRights::READ,
            Invocation::Socket(SocketOp::Create { .. }) => AccessRights::CREATE,
        }
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HandleID(pub usize);

define_bitflags! {
    pub struct AccessRights(u8) {
        READ            = 1 << 0;
        WRITE           = 1 << 1;
        EXECUTE         = 1 << 2;
        CREATE          = 1 << 3;
        MUTATE          = 1 << 4;
    }
}

pub struct ProcStatus {
    pub pid: usize,
    pub active_threads: usize,
    pub is_terminated: bool,
    pub memory_usage: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HandleGrant {
    pub id: HandleID,
    pub rights: AccessRights,
    pub tag: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProcessInitPackage {
    pub self_handle: HandleID,
    pub root_handle: HandleID,
    pub source_handle: HandleID,
    pub sink_handle: HandleID,

    pub extra_handles_ptr: *const HandleGrant,
    pub extra_handles_len: usize,

    pub argc: usize,
    pub argv: *const *const u8,
}

impl ProcessInitPackage {
    pub fn ext(&self) -> &[HandleGrant] {
        unsafe { slice::from_raw_parts(self.extra_handles_ptr, self.extra_handles_len) }
    }
}
