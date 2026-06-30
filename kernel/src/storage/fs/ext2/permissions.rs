use vespertine_abi::{
    AccessRights,
    UserID,
};

use crate::security::permissions::FilePermissions;

fn owner_bits(mode: u16) -> u16 { (mode >> 6) & 0b111 }

fn other_bits(mode: u16) -> u16 { mode & 0b111 }

fn file_rights(bits: u16) -> AccessRights {
    let mut rights = AccessRights::new();

    if bits & 0b100 != 0 {
        rights = rights | AccessRights::READ;
    }
    if bits & 0b010 != 0 {
        rights = rights | AccessRights::WRITE;
    }
    if bits & 0b001 != 0 {
        rights = rights | AccessRights::EXECUTE
    }
    rights
}

fn directory_rights(bits: u16) -> AccessRights {
    let mut rights = AccessRights::new();

    if bits & 0b100 != 0 {
        rights = rights | AccessRights::LIST;
    }
    if bits & 0b010 != 0 {
        rights = rights | AccessRights::CREATE | AccessRights::REMOVE;
    }
    if bits & 0b001 != 0 {
        rights = rights | AccessRights::TRAVERSE
    }
    rights
}

pub fn file_permissions(uid: u16, mode: u16) -> FilePermissions {
    FilePermissions { owner: UserID(uid as u32), owner_rights: file_rights(owner_bits(mode)), other_rights: file_rights(other_bits(mode)) }
}

pub fn directory_permissions(uid: u16, mode: u16) -> FilePermissions {
    FilePermissions {
        owner: UserID(uid as u32),
        owner_rights: directory_rights(owner_bits(mode)),
        other_rights: directory_rights(other_bits(mode)),
    }
}
