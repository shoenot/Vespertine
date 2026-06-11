use super::*;
use core::fmt::Debug;

#[repr(C)]
#[derive(Debug)]
pub enum ChannelOp {
    PushSmall {
        data: [u8; 64],
        len: u8,
    },
    PushLarge {
        vmo_handle: HandleID,
        offset: usize,
        len: usize,
    },
    Pull {
        buffer_ptr: usize,
    },
}

#[repr(C)]
#[derive(Debug)]
pub enum SocketOp {
    Create {
        sourceproc: HandleID,
        sinkproc: HandleID,
    },
    SetNB {
        nb: bool,
    }, // non blocking not non binary. but could be non binary. up to u.
    SetReadPolicy {
        min: usize,
        timeout_ds: usize,
    },
}

#[repr(C)]
#[derive(Debug)]
pub enum DirectoryOp {
    Link {
        name: usize,
        name_len: usize,
        handle_id: HandleID,
    },
    Unlink {
        name: usize,
        name_len: usize,
    },
    Lookup {
        name: usize,
        name_len: usize,
    },
    List {
        offset: usize,
        sink: HandleID,
    },
    CreateFile {
        name: usize,
        name_len: usize,
    },
    CreateDir {
        name: usize,
        name_len: usize,
    },
}

#[repr(C)]
#[derive(Debug)]
pub enum FileOp {
    Read {
        offset: usize,
        buffer_ptr: usize,
        len: usize,
    },
    Write {
        offset: usize,
        buffer_ptr: usize,
        len: usize,
    },
    Stat,
    GetVmo,
    Seek {
        offset: i64,
        whence: u32,
    },
    Truncate {
        size: usize,
    },
}

#[repr(C)]
#[derive(Debug)]
pub enum VmoOp {
    GetPage {
        offset: usize,
    },
    Resize {
        new_size: usize,
    },
    Clone {
        offset: usize,
        len: usize,
    },
    MapIntoProc {
        vaddr: usize,
        len: usize,
        vm_flags: usize,
    },
}

#[repr(C)]
#[derive(Debug)]
pub enum ProcOp {
    Kill,
    GetStatus {
        status_ptr: usize,
    },
    Unmap {
        vaddr: usize,
        len: usize,
    },
    SpawnThread {
        entry: usize,
        stack_top: usize,
        arg: usize,
        priority: u8,
    },
    SetFsBase {
        fs_base: usize,
    },
    InsertHandle {
        source_handle: HandleID,
        rights: AccessRights,
    },
    Mprotect {
        vaddr: usize,
        len: usize,
        prot: usize,
    },
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

        capabilities_ptr: usize,
        capabilities_len: usize,

        args_buffer_ptr: usize,
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
    RequestExpansion { additional_bytes: usize },
}

#[repr(C)]
#[derive(Debug)]
pub enum ClockOp {
    GetTimestamp { s_ptr: usize, ns_ptr: usize },
    Sleep { ns: usize },
}

#[repr(C)]
#[derive(Debug)]
pub enum WaitOp {
    One(Signal),
    Many { items_ptr: usize, count: usize },
}

#[repr(C)]
#[derive(Debug)]
pub enum BrokerOp {
    Request {
        capability: CapabilityID,
        requested_rights: AccessRights,
    },
}
