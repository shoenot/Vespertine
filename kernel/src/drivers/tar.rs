use core::str::from_utf8;

use alloc::sync::Arc;

use crate::{MODULE_REQUEST, core::object::{handle::{AccessRights, HandleID}, invoke::InvocationError, models::{directory::Directory, file::FileObj}, vfs::{ROOT_DIRECTORY, kernel_close, kernel_register_obj, kernel_walk, mount_kernel_dir}}};

#[repr(C)]
struct TarHeader {
    filename:   [u8; 100],
    mode:       [u8; 8],
    uid:        [u8; 8],
    gid:        [u8; 8],
    size:       [u8; 12],
    mtime:      [u8; 12],
    chksum:     [u8; 8],
    typeflag:    u8,
}

pub fn get_ramdisk_ptr() -> *const u8 {
    let response = MODULE_REQUEST.response().unwrap();
    let ramdisk = response.modules()[0];
    ramdisk.data().as_ptr()
}

pub fn get_ramdisk_size() -> usize {
    let response = MODULE_REQUEST.response().unwrap();
    response.modules()[0].data().len()
}

pub fn parse_tar(ptr: *const u8, size: usize) -> Result<(), InvocationError> {
    unsafe {
        let mut offset = 0;
        while offset < size {
            let header_ptr = (ptr.add(offset)) as *const TarHeader;
            let header = &*header_ptr;

            if header.filename[0] == 0 { break; }
            let filename_len = header.filename.iter().position(|&c| c == 0).unwrap_or(100);
            let filename_str = from_utf8(&header.filename[..filename_len]).unwrap();

            let size_len = header.size.iter().position(|&c| c == 0 || c == b' ').unwrap_or(12);
            let size_str = from_utf8(&header.size[..size_len]).unwrap();

            let file_size = usize::from_str_radix(size_str.trim(), 8).unwrap();

            let path_trimmed = filename_str.trim_end_matches('/');
            if path_trimmed == "." || path_trimmed == ".." || path_trimmed.is_empty() {
                offset += 512 + ((file_size + 511) & !511);
                continue;
            }

            let (parent_path, child_name) = match path_trimmed.rfind('/') {
                Some(idx) => (&path_trimmed[..idx], &path_trimmed[idx + 1..]),
                None => ("", path_trimmed),
            };

            match header.typeflag {
                b'5' => {
                    let parent_handle = kernel_walk(parent_path, HandleID(0))?;
                    let new_dir_handle = kernel_register_obj(Arc::new(Directory::new()), AccessRights::all());
                    mount_kernel_dir(child_name, new_dir_handle, parent_handle);
                    if parent_handle != HandleID(0) { let _ = kernel_close(parent_handle); }
                    let _ = kernel_close(new_dir_handle);
                },
                b'0' | b'\0' => {
                    let parent_handle = kernel_walk(parent_path, HandleID(0))?;
                    let file_ptr = ptr.add(offset + 512);
                    let file_handle = kernel_register_obj(Arc::new(FileObj::new(file_ptr, file_size)), AccessRights::all());
                    mount_kernel_dir(child_name, file_handle, parent_handle);
                    if parent_handle != HandleID(0) { let _ = kernel_close(parent_handle); }
                    let _ = kernel_close(file_handle);
                }
                _ => return Err(InvocationError::InvalidArgument),
            }


            offset += 512 + ((file_size + 511) & !511);
        }
    }
    Ok(())
}
