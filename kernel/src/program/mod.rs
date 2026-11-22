pub mod env;
pub mod parser;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use hal::mmu::PageSize;
use core::mem::MaybeUninit;
use core::fmt;

use parser::*;
use vespertine_abi::{
    AccessRights,
    FileOp,
    FileStat,
    HandleID,
    Invocation,
};

use crate::object::invoke::InvocationError;
use crate::process::Process;
use crate::object::obj::KernelObject;
use crate::object::vfs::kernel_walk;
use crate::process::current_process;
use crate::klogln;
use crate::memory::vmm::{
    CachePolicy, MapBehavior, USER_NULL_GUARD_END, USER_SPACE_LIMIT, VmOptions, VmPermissions, VmaBacking, VmaChargeKind, 
};
use crate::memory::vmo::AnonymousVmo;
use crate::memory::{
    PAGE_SIZE_4K,
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
    OutOfMemory,
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
            LoaderError::OutOfMemory => write!(f, "Out of memory"),
        }
    }
}

impl From<InvocationError> for LoaderError {
    fn from(value: InvocationError) -> Self {
        match value {
            InvocationError::OutOfMemory => Self::OutOfMemory,
            _ => Self::FileReadError,
        }
    }
}

pub struct ElfLoadResult {
    pub entry_point: usize,
    pub phdr_addr: usize,
    pub phnum: usize,
    pub base_addr: usize,
    pub interpreter_entry: Option<usize>,
}

async fn read_elf_header(file_obj: &Arc<dyn KernelObject>) -> Result<(Vec<u8>, Elf64_Ehdr), LoaderError> {
    let mut stat = MaybeUninit::<FileStat>::zeroed();
    file_obj.invoke(Invocation::File(
        FileOp::Stat { stat_ptr: stat.as_mut_ptr() as usize }),
        AccessRights::READ,
    ).await
        .map_err(|error| {
            klogln!("[ERROR] read_elf_header: Stat failed: {:?}", error);
            LoaderError::FileReadError
        })?;

    let file_size = unsafe { stat.assume_init().size as usize };
    let header_len = file_size.min(PAGE_SIZE_4K);
    if header_len == 0 { return Err(LoaderError::InvalidBuffer); }

    let backing = AnonymousVmo::new(header_len);
    file_obj.copy_file_into_vmo(0, header_len, &backing)
        .await
        .map_err(|error| {
            klogln!("[ERROR] read_elf_header: Copy failed: {:?}", error);
            LoaderError::FileReadError
        })?;

    let frame = backing.get_page(0).map_err(|_| LoaderError::FileReadError)?;
    let header_bytes = unsafe {
        core::slice::from_raw_parts(frame.virt() as *const u8, header_len)
    }.to_vec();

    let header = *Elf64_Ehdr::from_bytes(&header_bytes)?;
    Ok((header_bytes, header))
}

async fn map_elf_segments(file_obj: &Arc<dyn KernelObject>, header: &Elf64_Ehdr, file_bytes: &[u8], proc: &Process, load_base: usize) -> Result<usize, LoaderError> {
    const PT_PHDR: u32 = 6;

    let page = PAGE_SIZE_4K;
    let ph_iter = header.prog_headers(file_bytes).ok_or(LoaderError::InvalidBuffer)?;
    let mut phdr_addr = 0;

    for ph in ph_iter {
        if ph.p_type == PT_PHDR {
            phdr_addr = ph.p_vaddr as usize;
        }
        if ph.p_type != P_Type::PT_LOAD as u32 {
            continue;
        }

        let file_size = ph.p_filesz as usize;
        let mem_size = ph.p_memsz as usize;
        if file_size > mem_size {
            return Err(LoaderError::InvalidBuffer);
        }
        if mem_size == 0 {
            continue;
        }

        let vaddr = load_base.checked_add(ph.p_vaddr as usize)
            .ok_or(LoaderError::InvalidBuffer)?;
        let file_offset = ph.p_offset as usize;
        let page_offset = vaddr % page;

        if file_offset % page != page_offset {
            return Err(LoaderError::InvalidBuffer);
        }

        let aligned_vaddr = vaddr - page_offset;
        let aligned_file_offset = file_offset - page_offset;
        let map_size = page_offset.checked_add(mem_size)
            .and_then(|size| size.checked_add(page - 1))
            .map(|size| size & !(page - 1))
            .ok_or(LoaderError::InvalidBuffer)?;

        let map_end = aligned_vaddr.checked_add(map_size)
            .ok_or(LoaderError::InvalidBuffer)?;
        if aligned_vaddr < USER_NULL_GUARD_END || map_end > USER_SPACE_LIMIT {
            return Err(LoaderError::InvalidBuffer);
        }

        let backing = AnonymousVmo::new(map_size);
        if file_size != 0 {
            // file_size <= mem_size, and page_offset + mem_size was checked above.
            let copy_bytes = page_offset + file_size;
            file_obj.copy_file_into_vmo(aligned_file_offset, copy_bytes, &backing)
                .await
                .map_err(|error| {
                    klogln!(
                        "[ERROR] load_elf: Copy at file offset {:#x} failed: {:?}",
                        aligned_file_offset,
                        error,
                    );
                    LoaderError::FileReadError
                })?;
        }

        let mut permissions = VmPermissions::USER;
        if ph.p_flags & PF_W != 0 {
            permissions = permissions | VmPermissions::WRITE;
        }
        if ph.p_flags & PF_X != 0 {
            permissions = permissions | VmPermissions::EXECUTE;
        }

        let options = VmOptions {
            permissions,
            max_permissions: permissions,
            cache: CachePolicy::Normal,
            page_size: PageSize::Size4K,
            charge: VmaChargeKind::Private,
        };

        let mut slot = proc.vmm.write();
        let vmm = slot.as_mut().ok_or(LoaderError::InvalidBuffer)?;
        vmm.map_at(
            aligned_vaddr,
            map_size,
            options,
            VmaBacking::Vmo(backing),
            0,
            MapBehavior::RequireVacant,
        )
        .map_err(|error| {
            klogln!(
                "[ERROR] load_elf: Map at {:#x} failed: {:?}",
                aligned_vaddr,
                error,
            );
            LoaderError::FileReadError
        })?;
    }

    if phdr_addr == 0 {
        let ph_iter = header.prog_headers(file_bytes).ok_or(LoaderError::InvalidBuffer)?;
        for ph in ph_iter {
            if ph.p_type == P_Type::PT_LOAD as u32 && ph.p_offset == 0 {
                phdr_addr = (ph.p_vaddr as usize)
                    .checked_add(header.e_phoff as usize)
                    .ok_or(LoaderError::InvalidBuffer)?;
                break;
            }
        }
    }

    Ok(phdr_addr)
}

pub async fn load_elf(file_obj: Arc<dyn KernelObject>, proc: &Process) -> Result<ElfLoadResult, LoaderError> {
    let (file_bytes, header) = read_elf_header(&file_obj).await?;

    // check if there is an interpreter (PT_INTERP) segment
    let ph_iter = header.prog_headers(&file_bytes).ok_or(LoaderError::InvalidBuffer)?;
    let mut interpreter_path: Option<String> = None;
    for ph in ph_iter {
        if ph.p_type == P_Type::PT_INTERP as u32 {
            let interp_offset = ph.p_offset as usize;
            let interp_len = ph.p_filesz as usize;
            if interp_offset + interp_len <= file_bytes.len() {
                let raw_path = &file_bytes[interp_offset..interp_offset + interp_len];
                let len = raw_path.iter().position(|&b| b == 0).unwrap_or(raw_path.len());
                if let Ok(path_str) = str::from_utf8(&raw_path[..len]) {
                    interpreter_path = Some(String::from(path_str));
                }
            }
        }
    }

    let main_phdr_addr = map_elf_segments(&file_obj, &header, &file_bytes, proc, 0).await?;

    let mut load_result = ElfLoadResult {
        entry_point: header.e_entry as usize,
        phdr_addr: main_phdr_addr,
        phnum: header.e_phnum as usize,
        base_addr: 0,
        interpreter_entry: None,
    };

    if let Some(path) = interpreter_path {
        let interp_handle = kernel_walk(&path, HandleID(0), AccessRights::READ).await.map_err(|e| {
            klogln!("[ERROR] load_elf: Failed to resolve interpreter path {}: {:?}", path, e);
            LoaderError::FileReadError
        })?;

        let interp_obj = {
            let caller = current_process().ok_or(LoaderError::FileReadError)?;
            let mut handles = caller.handles.write();

            let object = handles.resolve(interp_handle, AccessRights::READ);
            let _ = handles.close(interp_handle);

            object.map_err(|_| LoaderError::FileReadError)?
        };

        let (interp_bytes, interp_header) = read_elf_header(&interp_obj).await?;

        let interp_base = 0x40_0000_0000;
        let _interp_phdr_addr = map_elf_segments(&interp_obj, &interp_header, &interp_bytes, proc, interp_base).await?;

        load_result.base_addr = interp_base;
        load_result.interpreter_entry = Some(interp_base + interp_header.e_entry as usize);
    }
    Ok(load_result)
}
