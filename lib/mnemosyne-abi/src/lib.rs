#![no_std]
#![no_main]
pub mod op;
mod bitwise;

use core::fmt::Debug;
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
