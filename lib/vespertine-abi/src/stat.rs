#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    File = 1,
    Directory = 2,
    Other = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FileStat {
    pub object_type: u32,
    pub mode: u32,
    pub user: u32,
    pub _group: u32,
    pub inode: u64,
    pub device: u64,
    pub size: u64,
    pub block_size: u32,
    pub blocks: u64,
    pub nlink: u32,
    pub atime_sec: i64,
    pub atime_nsec: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
}

impl FileStat {
    pub fn zeroed() -> FileStat {
        FileStat {
            object_type: 0,
            mode: 0,
            user: 0,
            _group: 0,
            inode: 0,
            device: 0,
            size: 0,
            block_size: 0,
            blocks: 0,
            nlink: 0,
            atime_sec: 0,
            atime_nsec: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            ctime_sec: 0,
            ctime_nsec: 0,
        }
    }
}
