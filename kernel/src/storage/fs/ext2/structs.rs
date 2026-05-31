use core::fmt::Debug;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DiskBlockPointers {
    pub direct: [u32; 12],
    pub single_indirect: u32,
    pub double_indirect: u32,
    pub triple_indirect: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub union FileData {
    pub blocks: DiskBlockPointers,
    pub symlink_embedded: [u8; 60],
}

impl Debug for FileData {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, "File Data Union") }
}

#[repr(C, packed)]
#[derive(Debug)]
pub struct DiskSuperblock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub r_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub log_frag_size: u32,
    pub blocks_per_group: u32,
    pub frags_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: u32,
    pub wtime: u32,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
    pub lastcheck: u32,
    pub checkinterval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub def_resuid: u16,
    pub def_resgid: u16,
    //-- EXT2_DYNAMIC_REV Specific --
    pub first_ino: u32,
    pub inode_size: u16,
    pub block_group_nr: u16,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub uuid: [u8; 16],
    pub volume_name: [u8; 16],
    pub last_mounted: [u8; 64],
    pub algo_bitmap: u32,
    //-- Performance Hints --
    pub prealloc_blocks: u8,
    pub prealloc_dir_blocks: u8,
    pub alignment: u16,
    //-- Journaling Support --
    pub journal_uuid: [u8; 16],
    pub journal_inum: u32,
    pub journal_dev: u32,
    pub last_orphan: u32,
    //-- Directory Indexing Support --
    pub hash_seed: [u32; 4],
    pub def_hash_version: u8,
    pub padding: [u8; 3],
    //-- Other options --
    pub default_mount_options: u32,
    pub first_meta_bg: u32,
    pub unused: [u8; 760],
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DiskGroupDesc {
    pub block_bitmap: u32,
    pub inode_bitmap: u32,
    pub inode_table: u32,
    pub free_blocks_count: u16,
    pub free_inodes_count: u16,
    pub used_dirs_count: u16,
    pub pad: u16,
    pub reserved: [u8; 12],
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DiskInode {
    pub mode: u16,
    pub uid: u16,
    pub size: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub links_count: u16,
    pub blocks: u32,
    pub flags: u32,
    pub osdl1: u32,
    pub data: FileData,
    pub generation: u32,
    pub file_acl: u32,
    pub dir_acl: u32,
    pub faddr: u32,
    pub osd2: [u8; 12],
}

#[repr(C, packed)]
pub struct DiskDirHeader {
    pub inode: u32,
    pub record_length: u16,
    pub name_length: u8,
    pub file_type: u8,
}
