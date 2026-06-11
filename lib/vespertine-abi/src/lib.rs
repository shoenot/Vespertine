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
            Invocation::Directory(DirectoryOp::Link { .. }) => AccessRights::WRITE,
            Invocation::Directory(DirectoryOp::Unlink { .. }) => AccessRights::WRITE,
            Invocation::Directory(DirectoryOp::Lookup { .. }) => AccessRights::READ,
            Invocation::Directory(DirectoryOp::List { .. }) => AccessRights::READ,
            Invocation::Directory(DirectoryOp::CreateFile { .. }) => AccessRights::WRITE,
            Invocation::Directory(DirectoryOp::CreateDir { .. }) => AccessRights::WRITE,
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
    }
}

define_bitflags! {
    pub struct Signal(u32) {
        READABLE    = 1 << 0;
        WRITABLE    = 1 << 1;
        PEER_CLOSED = 1 << 2;
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

#[repr(C)]
#[derive(Debug, Clone)]
pub struct WaitItem {
    pub handle: HandleID,
    pub signal: Signal,
    pub pending: Signal,
}
