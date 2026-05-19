use crate::kernel::object::handle::HandleID;

#[repr(C)]
#[derive(Debug)]
pub enum ChannelOp {
    PushSmall { data: [u8; 32], len: u8 },
    PushLarge { vmo_handle: HandleID, offset: usize, len: usize },
    Pull,
}

#[repr(C)]
#[derive(Debug)]
pub enum DirectoryOp {
    Link { name: *const u8, name_len: usize, handle_id: HandleID },
    Unlink { name: *const u8, name_len: usize },
    Lookup { name: *const u8, name_len: usize },
}

#[repr(C)]
#[derive(Debug)]
pub enum FileOp {
    Read { offset: usize, buffer_ptr: *mut u8, len: usize },
}
