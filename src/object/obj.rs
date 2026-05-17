use crate::object::handle::HandleID;


pub enum ChannelMessage {
    Small { data: [u8; 32], len: u8 },
    Large { vmo_handle: HandleID, offset: usize, len: usize }
}

pub enum Invocation {
    Ping,
    GetInfo,
}

pub enum InvocationError {
    AccessDenied,
    InvalidHandle,
}

trait KernelObject {
    fn invoke(&self, invocation: Invocation) -> Result<(), InvocationError>;
}


