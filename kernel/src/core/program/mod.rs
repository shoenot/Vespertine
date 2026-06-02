pub mod env;
pub mod parser;
use alloc::alloc::{
    Layout,
    alloc,
};
use alloc::sync::Arc;
use core::ptr::copy_nonoverlapping;
use core::slice::from_raw_parts;
use core::{
    cmp,
    fmt,
};

use parser::*;
use vespertine_abi::{
    AccessRights,
    FileOp,
    HandleID,
    Invocation,
};

use crate::arch::get_core_data;
use crate::core::object::models::process::Process;
use crate::core::object::models::vmo::VmoObject;
use crate::core::thread::{
    ThreadControlBlock,
    get_current_process,
};
use crate::memory::{HHDMOFFSET, NORMAL_PAGE_SIZE};
use crate::memory::vmm::{
    VM_FLAG_EXEC,
    VM_FLAG_USER,
    VM_FLAG_WRITE,
    align_up,
};
use crate::memory::vmo::{
    PagedBackingStore,
    Vmo,
};
use crate::{
    KERNEL_PROCESS,
    klogln,
};

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

pub async fn load_elf(file_handle: HandleID, proc: &Process) -> Result<(usize, usize, usize), LoaderError> {
    // IN USER THREAD CONTEXT
    let file_obj =
        get_current_process().ok_or(LoaderError::FileReadError)?.proc_handles.read().resolve(file_handle, AccessRights::READ).map_err(
            |e| {
                klogln!("[ERROR] load_elf: Failed to resolve file_handle: {:?}", e);
                LoaderError::FileReadError
            },
        )?;

    // SWITCH TO KERNEL PROCESS TEMPORARILY
    let current_thread = get_core_data().scheduler.get_current_thread();

    let thread_addr = current_thread as usize;
    let old_proc = unsafe { (*current_thread).process.clone() };

    unsafe {
        (*current_thread).process = KERNEL_PROCESS.get().unwrap().clone();
    }

    // read only first 4k to parse headers
    let file_size = file_obj.invoke(Invocation::File(FileOp::Stat), AccessRights::READ).await.map_err(|e| {
        klogln!("[ERROR] load_elf: Stat failed: {:?}", e);
        LoaderError::FileReadError
    })?;
    let header_read_size = cmp::min(file_size, 4096);

    let file_layout = Layout::from_size_align(header_read_size, 8).map_err(|_| LoaderError::FileReadError)?;
    let buffer_ptr = unsafe { alloc(file_layout) as *mut u8 };
    let buf_addr = buffer_ptr as usize;

    let read_result = file_obj
        .invoke(Invocation::File(FileOp::Read { offset: 0, buffer_ptr: buffer_ptr as usize, len: header_read_size }), AccessRights::READ);

    // RESTORE USER PROCESS TO DROP PRIVILEGES
    let thread_ptr = thread_addr as *mut ThreadControlBlock;
    unsafe {
        (*thread_ptr).process = old_proc;
    }

    read_result.await.map_err(|e| {
        klogln!("[ERROR] load_elf: Read failed: {:?}", e);
        LoaderError::FileReadError
    })?;
    let file_bytes = unsafe { from_raw_parts(buf_addr as *mut u8, header_read_size) };

    let header = Elf64_Ehdr::from_bytes(file_bytes)?;
    let ph_iter = header.prog_headers(file_bytes).unwrap();

    let vmo_handle_id = file_obj.invoke(Invocation::File(FileOp::GetVmo), AccessRights::READ).await.map_err(|e| {
        klogln!("[ERROR] load_elf: GetVmo failed: {:?}", e);
        LoaderError::FileReadError
    })?;
    let vmo_handle = HandleID(vmo_handle_id);
    let current_proc = get_current_process().ok_or(LoaderError::FileReadError)?;
    let vmo_obj_dyn = current_proc.proc_handles.read().resolve(vmo_handle, AccessRights::READ).map_err(|e| {
        klogln!("[ERROR] load_elf: Resolve VmoObject handle failed: {:?}", e);
        LoaderError::FileReadError
    })?;
    let vmo_obj = vmo_obj_dyn.as_any().downcast_ref::<VmoObject>().ok_or_else(|| {
        klogln!("[ERROR] load_elf: Downcast to VmoObject failed");
        LoaderError::FileReadError
    })?;
    let file_vmo = vmo_obj.vmo.clone();
    let _ = current_proc.proc_handles.write().close(vmo_handle);

    let mut phdr_addr = 0;
    for ph in ph_iter {
        if ph.p_type == 6 {
            // PT_PHDR
            phdr_addr = ph.p_vaddr as usize;
        }

        if ph.p_type == P_Type::PT_LOAD as u32 {
            klogln!(
                "[INFO] Mapping Segment: file offset 0x{:X} -> virt addr 0x{:X} file size: {}, mem_size: {}",
                ph.p_offset,
                ph.p_vaddr,
                ph.p_filesz,
                ph.p_memsz
            );

            let aligned_vaddr = (ph.p_vaddr & !0xFFF) as usize;
            let aligned_offset = (ph.p_offset & !0xFFF) as usize;
            let offset_in_page = (ph.p_vaddr & 0xFFF) as usize;
            let total_map_size = align_up(offset_in_page + ph.p_memsz as usize);

            let mut vm_flags = VM_FLAG_USER;
            if (ph.p_flags & PF_W) != 0 {
                vm_flags |= VM_FLAG_WRITE
            };
            if (ph.p_flags & PF_X) != 0 {
                vm_flags |= VM_FLAG_EXEC
            };

            let (segment_vmo, map_offset) = if ph.p_filesz == 0 {
                (Vmo::new(total_map_size) as Arc<dyn PagedBackingStore>, 0)
            } else if ph.p_memsz as usize > ph.p_filesz as usize {
                let anon_vmo = Vmo::new(total_map_size);
                
                let mut progress = 0;
                while progress < ph.p_filesz as usize {
                    let file_offset = aligned_offset + progress;
                    let target_offset = offset_in_page + progress;
                    
                    let file_pfn = file_vmo.request_page(file_offset).map_err(|_| LoaderError::FileReadError)?;
                    
                    let anon_pfn = anon_vmo.request_page(target_offset).map_err(|_| LoaderError::FileReadError)?;
                    
                    let src_virt = file_pfn + *HHDMOFFSET;
                    let dest_virt = anon_pfn + *HHDMOFFSET;
                    
                    unsafe {
                        copy_nonoverlapping(
                            src_virt as *const u8,
                            dest_virt as *mut u8,
                            NORMAL_PAGE_SIZE,
                        );
                    }
                    progress += NORMAL_PAGE_SIZE;
                }
                (anon_vmo as Arc<dyn PagedBackingStore>, 0)
            } else {
                (file_vmo.clone(), aligned_offset)
            };

            proc.vmm.write().mmap_vmo_at(aligned_vaddr, total_map_size, vm_flags, segment_vmo, map_offset).ok_or_else(|| {
                klogln!("[ERROR] load_elf: mmap_vmo_at failed for segment at 0x{:X}", aligned_vaddr);
                LoaderError::FileReadError
            })?;
        }
    }

    if phdr_addr == 0 {
        for ph in header.prog_headers(file_bytes).unwrap() {
            if ph.p_type == 1 && ph.p_offset == 0 {
                // PT_LOAD
                phdr_addr = (ph.p_vaddr + header.e_phoff) as usize;
                break;
            }
        }
    }

    klogln!("[INFO] Ready to jump to entry 0x{:X}", header.e_entry);
    Ok((header.e_entry as usize, phdr_addr, header.e_phnum as usize))
}
