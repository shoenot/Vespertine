use crate::define_bitflags;


pub static VESPER_MAGIC: u32 = 0xc001ca75;  // cool cats

#[repr(C, align(8))]
pub struct PacketHeader {
    pub magic: u32,
    pub version: u16,
    pub packet_flags: PacketFlags,
    pub packet_type: u32,
    pub payload_len: u32,
    pub reserved: u32,  // padding to make header 24 bytes
}

#[repr(C)]
pub enum PacketType {
    Error = 0,
    DirEntry = 1,
    ProcessInfo = 2,
    MemoryInfo = 3,
    HandleInfo = 4,
    SystemLog = 5,
}

define_bitflags! {
    pub struct PacketFlags(u16) {
        // packet is a single, complete buffer.
        IS_BUFFER     = 1 << 0;
        // packet is one item in a stream of packet.
        IS_STREAM     = 1 << 1;
        // there are more packets coming after this one.
        HAS_NEXT      = 1 << 2;
        // the payload contains a 'count' field at the start.
        HAS_COUNT     = 1 << 3;
    }
}

pub struct AbiDirEntry {
    pub entry_type: u8,
    pub name_len: u8,
    pub name: [u8; 254],
}

pub enum DirEntryType {
    Unknown = 0,
    Directory = 1,
    File = 2,
    Object = 3,
}
