#![no_std]
#![no_main]
pub mod app;
mod bitwise;
pub mod op;
pub mod protocol;
pub mod tag;

pub const AT_VESPERTINE_INITPKG: usize = 0x6fff_0001;

use core::{fmt::Debug, slice};
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
    Thread(ThreadOp),
    ProcessManager(ProcManOp),
    MemoryManager(MemManOp),
    Broker(BrokerOp),
    MemPool(MemPoolOp),
    Clock(ClockOp),
    Socket(SocketOp),
    Wait(WaitOp),
}

impl Invocation {
    pub fn required_rights(&self) -> AccessRights {
        match self {
            Invocation::Ping => AccessRights::READ,
            Invocation::GetInfo => AccessRights::READ,
            Invocation::Channel(ChannelOp::PushSmall { .. }) => AccessRights::WRITE,
            Invocation::Channel(ChannelOp::PushLarge { .. }) => AccessRights::WRITE,
            Invocation::Channel(ChannelOp::Pull { .. }) => AccessRights::READ,
            Invocation::Directory(DirectoryOp::Lookup { .. }) => AccessRights::TRAVERSE,
            Invocation::Directory(DirectoryOp::Resolve { .. }) => AccessRights::TRAVERSE,
            Invocation::Directory(DirectoryOp::List { .. }) => AccessRights::LIST,
            Invocation::Directory(DirectoryOp::Link { .. }) => AccessRights::CREATE,
            Invocation::Directory(DirectoryOp::CreateFile { .. }) => AccessRights::CREATE,
            Invocation::Directory(DirectoryOp::CreateDir { .. }) => AccessRights::CREATE,
            Invocation::Directory(DirectoryOp::Unlink { .. }) => AccessRights::REMOVE,
            Invocation::File(FileOp::Read { .. }) => AccessRights::READ,
            Invocation::File(FileOp::Write { .. }) => AccessRights::WRITE,
            Invocation::File(FileOp::Stat) => AccessRights::new(),
            Invocation::File(FileOp::GetVmo) => AccessRights::READ,
            Invocation::File(FileOp::Seek { .. }) => AccessRights::new(),
            Invocation::File(FileOp::Truncate { .. }) => AccessRights::WRITE,
            Invocation::Vmo(VmoOp::GetPage { .. }) => AccessRights::READ,
            Invocation::Vmo(VmoOp::Resize { .. }) => AccessRights::MUTATE,
            Invocation::Vmo(VmoOp::Clone { .. }) => AccessRights::CREATE,
            Invocation::Vmo(VmoOp::MapIntoProc { .. }) => AccessRights::MUTATE,
            Invocation::Proc(ProcOp::Kill) => AccessRights::WRITE,
            Invocation::Proc(ProcOp::GetStatus { .. }) => AccessRights::READ,
            Invocation::Proc(ProcOp::GetExitInfo { .. }) => AccessRights::READ,
            Invocation::Proc(ProcOp::Unmap { .. }) => AccessRights::MUTATE,
            Invocation::Proc(ProcOp::SpawnThread { .. }) => AccessRights::CREATE,
            Invocation::Proc(ProcOp::SetFsBase { .. }) => AccessRights::WRITE,
            Invocation::Proc(ProcOp::InsertHandle { .. }) => AccessRights::MUTATE,
            Invocation::Proc(ProcOp::Mprotect { .. }) => AccessRights::MUTATE,
            Invocation::Thread(ThreadOp::Kill) => AccessRights::WRITE,
            Invocation::Thread(ThreadOp::Join) => AccessRights::READ,
            Invocation::Thread(ThreadOp::GetID) => AccessRights::READ,
            Invocation::ProcessManager(ProcManOp::Spawn { .. }) => AccessRights::CREATE,
            Invocation::MemoryManager(MemManOp::CreatePool { .. }) => AccessRights::CREATE,
            Invocation::MemPool(MemPoolOp::AllocateVmo { .. }) => AccessRights::CREATE,
            Invocation::MemPool(MemPoolOp::CreateSubPool { .. }) => AccessRights::CREATE,
            Invocation::MemPool(MemPoolOp::RequestExpansion { .. }) => AccessRights::MUTATE,
            Invocation::Clock(ClockOp::GetTimestamp { .. }) => AccessRights::READ,
            Invocation::Clock(ClockOp::Sleep { .. }) => AccessRights::WRITE,
            Invocation::Socket(SocketOp::Create { .. }) => AccessRights::CREATE,
            Invocation::Socket(SocketOp::SetNB { .. }) => AccessRights::WRITE,
            Invocation::Socket(SocketOp::SetReadPolicy { .. }) => AccessRights::WRITE,
            Invocation::Wait(..) => AccessRights::READ,
            Invocation::Broker(BrokerOp::Request { .. }) => AccessRights::READ,
        }
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HandleID(pub usize);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityID(pub usize);

define_bitflags! {
    pub struct AccessRights(u8) {
        READ            = 1 << 0;
        WRITE           = 1 << 1;
        EXECUTE         = 1 << 2;
        CREATE          = 1 << 3;
        MUTATE          = 1 << 4;
        TRAVERSE        = 1 << 5;
        LIST            = 1 << 6;
        REMOVE          = 1 << 7;
    }
}

define_bitflags! {
    pub struct Signal(u32) {
        READABLE    = 1 << 0;
        WRITABLE    = 1 << 1;
        PEER_CLOSED = 1 << 2;
        TERMINATED  = 1 << 3;
    }
}

pub struct ProcStatus {
    pub pid: usize,
    pub user: UserID,
    pub active_threads: usize,
    pub is_terminated: bool,
    pub memory_usage: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CapabilityGrant {
    pub id: HandleID,
    pub rights: AccessRights,
    pub capability: CapabilityID,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProcessInitPackage {
    pub self_handle: HandleID,
    pub root_handle: HandleID,
    pub source_handle: HandleID,
    pub sink_handle: HandleID,
    pub memory_pool_handle: HandleID,
    pub cwd_handle: HandleID,

    pub capabilities_ptr: *const CapabilityGrant,
    pub capabilities_len: usize,

    pub argc: usize,
    pub argv: *const *const u8,
    pub envp: *const *const u8,
}

impl ProcessInitPackage {
    pub fn capabilities(&self) -> &[CapabilityGrant] {
        unsafe { slice::from_raw_parts(self.capabilities_ptr, self.capabilities_len) }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessExitKind {
    Running = 0,
    Exited = 1,
    Killed = 2,
    Faulted = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProcessExitInfo {
    pub kind: ProcessExitKind,
    pub code: u32,
    pub detail: u64,
}

impl ProcessExitInfo {
    pub const fn running() -> Self {
        Self { kind: ProcessExitKind::Running, code: 0, detail: 0 }
    }

    pub const fn exited(code: u32) -> Self {
        Self { kind: ProcessExitKind::Exited, code, detail: 0 }
    }

    pub const fn killed(reason: u32) -> Self {
        Self { kind: ProcessExitKind::Killed, code: reason, detail: 0 }
    }
    
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct WaitItem {
    pub handle: HandleID,
    pub signal: Signal,
    pub pending: Signal,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct UserID(pub u32);

pub const SYSTEM_USER: UserID = UserID(0);
