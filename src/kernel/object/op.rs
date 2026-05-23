use crate::kernel::object::{handle::{AccessRights, HandleID}, models::process::ProcStatus};

#[repr(C)]
#[derive(Debug)]
pub enum ChannelOp {
    PushSmall { data: [u8; 64], len: u8 }, 
    PushLarge { vmo_handle: HandleID, offset: usize, len: usize },
    Pull { buffer_ptr: *mut u8 },
}

#[repr(C)]
#[derive(Debug)]
pub enum DirectoryOp {
    Link { name: *const u8, name_len: usize, handle_id: HandleID },
    Unlink { name: *const u8, name_len: usize },
    Lookup { name: *const u8, name_len: usize },
    List(usize),
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
}

#[repr(C)]
#[derive(Debug)]
pub enum ProcOp {
    Kill,
    GetStatus { status_ptr: *mut ProcStatus },
}


#[repr(C)]
#[derive(Debug)]
pub enum ProcManOp {
    Spawn { exec_handle: HandleID, root_handle: HandleID, root_rights: AccessRights },
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
