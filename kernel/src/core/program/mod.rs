pub mod parser;
use core::fmt;
use parser::*;

use alloc::alloc::{alloc, Layout};
use alloc::sync::Arc;
use core::slice::from_raw_parts;
use core::intrinsics::{copy_nonoverlapping, write_bytes};

use crate::klogln;
use crate::core::object::handle::HandleID;
use crate::core::object::invoke::Invocation;
use crate::core::object::op::FileOp;
use crate::core::object::vfs::kernel_invoke;
use crate::core::object::models::process::Process;
use crate::memory::{HHDMOFFSET, NORMAL_PAGE_SIZE};
use crate::memory::vmm::{align_up, VM_FLAG_EXEC, VM_FLAG_USER, VM_FLAG_WRITE};
use crate::memory::vmo::{PagedBackingStore, Vmo};

#[derive(Debug)]
pub enum LoaderError {
    InvalidBuffer,
    InvalidMagicNumbers,
    NotAWashingMachine,
    Not64BitElf,
    UnsupportedElfType(u16),
    UnsupportedArch(u16),
    UnsupportedABI(u8),
    FileReadError,
}

impl fmt::Display for LoaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoaderError::InvalidBuffer => write!(f, "InvalidBuffer"),
            LoaderError::InvalidMagicNumbers => write!(f, "Invalid ELF Magic numbers"),
            LoaderError::NotAWashingMachine => write!(f, "Big endian not supported"),
            LoaderError::Not64BitElf => write!(f, "32 bit programs not supported"),
            LoaderError::UnsupportedElfType(t) => write!(f, "Unsupported ELF type: 0x{:X}", t),
            LoaderError::UnsupportedArch(t) => write!(f, "Unsupported architechture: 0x{:X}", t),
            LoaderError::UnsupportedABI(t) => write!(f, "Unsupported ABI: 0x{:X}", t),
            LoaderError::FileReadError => write!(f, "File read or map error"),
        }
    }
}

pub fn load_elf(file_handle: HandleID, proc: &Process) -> Result<usize, LoaderError> {
    let file_size = kernel_invoke(file_handle, Invocation::File(FileOp::Stat))
        .map_err(|_| LoaderError::FileReadError)?;
    let file_layout = Layout::from_size_align(file_size, 8)
        .map_err(|_| LoaderError::FileReadError)?;

    let buffer_ptr = unsafe { alloc(file_layout) as *mut u8 };
    kernel_invoke(file_handle, Invocation::File(FileOp::Read { offset: 0, buffer_ptr, len: file_size }))
        .map_err(|_| LoaderError::FileReadError)?;
    let file_bytes = unsafe { from_raw_parts(buffer_ptr, file_size) };

    let header = Elf64_Ehdr::from_bytes(file_bytes)?;
    let ph_iter = header.prog_headers(file_bytes).unwrap();
    
    for ph in ph_iter {
        if ph.p_type == P_Type::PT_LOAD as u32 {
            klogln!(
                "Mapping Segment: file offset 0x{:X} -> virt addr 0x{:X} file size: {}, mem_size: {}",
                ph.p_offset, ph.p_vaddr, ph.p_filesz, ph.p_memsz
            );

            let aligned_vaddr = (ph.p_vaddr & !0xFFF) as usize;
            let offset_in_first_page = (ph.p_vaddr & 0xFFF) as usize;
            let total_map_size = align_up(offset_in_first_page + ph.p_memsz as usize);
            let vmo = Vmo::new(total_map_size as usize);

            let mut page_offset = 0;
            while page_offset < total_map_size {
                let pfn = vmo.request_page(page_offset).map_err(|_| LoaderError::FileReadError)?;
                let hhdm_ptr = pfn + *HHDMOFFSET;
                unsafe { write_bytes(hhdm_ptr as *mut u8, 0, NORMAL_PAGE_SIZE) };

                let overlap_start = usize::max(page_offset, offset_in_first_page as usize);
                let overlap_end = usize::min(page_offset + NORMAL_PAGE_SIZE, offset_in_first_page + ph.p_filesz as usize);

                if overlap_start < overlap_end {
                    unsafe {
                        let dst = ((overlap_start - page_offset) + hhdm_ptr) as *mut u8;
                        let src = file_bytes.as_ptr().add(ph.p_offset as usize + (overlap_start - offset_in_first_page));
                        let len = overlap_end - overlap_start;
                        copy_nonoverlapping(src, dst, len);
                    }
                }
                page_offset += NORMAL_PAGE_SIZE;
            }

            let mut vm_flags = VM_FLAG_USER;
            if (ph.p_flags & PF_W) != 0 { vm_flags |= VM_FLAG_WRITE };
            if (ph.p_flags & PF_X) != 0 { vm_flags |= VM_FLAG_EXEC };

            proc.vmm.write()
                .mmap_vmo_at(aligned_vaddr, total_map_size, vm_flags, vmo.clone())
                .ok_or(LoaderError::FileReadError)?;
        }
    }

    klogln!("Ready to jump to entry 0x{:X}", header.e_entry);
    Ok(header.e_entry as usize)
}
