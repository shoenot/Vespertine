use alloc::sync::Arc;
use core::ptr::{
    copy_nonoverlapping,
    null,
    write,
};

use vespertine_abi::{
    HandleGrant,
    ProcessInitPackage,
};
use vespertine_common::slab::NORMAL_PAGE_SIZE;

use crate::core::object::invoke::InvocationError;
use crate::memory::HHDMOFFSET;
use crate::memory::vmo::{
    PagedBackingStore,
    Vmo,
};

pub struct ProcessEnvironment;

#[repr(C)]
struct AuxEntry {
    a_type: usize,
    a_val: usize,
}

impl ProcessEnvironment {
    pub fn inject(
        stack_vmo: &Arc<Vmo>, stack_vaddr: usize, stack_size: usize, extra_handles: &[HandleGrant], args_buffer: &[u8], argc: usize,
        mut initpkg: ProcessInitPackage, entry_point: usize, phdr_addr: usize, phnum: usize,
    ) -> Result<(usize, usize), InvocationError> {
        let top_page_offset = stack_size - NORMAL_PAGE_SIZE;
        let phys_frame = stack_vmo.request_page(top_page_offset).map_err(|_| InvocationError::OutOfMemory)?;

        // calculate sizes
        let initpkg_size = size_of::<ProcessInitPackage>();
        let handles_array_size = extra_handles.len() * size_of::<HandleGrant>();
        let argv_array_size = (argc + 1) * size_of::<*const u8>(); // +1 for null terminator
        let strings_size = args_buffer.len();

        // aux vector (AT_PHDR, AT_PHNUM, AT_PHENT, AT_PAGESZ, AT_ENTRY, AT_NULL)
        let sysv_total_size = size_of::<usize>() + argv_array_size + size_of::<*const u8>() + 6 * size_of::<AuxEntry>();

        let total_payload_size = initpkg_size + handles_array_size + argv_array_size + strings_size + sysv_total_size;
        if total_payload_size > NORMAL_PAGE_SIZE {
            return Err(InvocationError::OutOfMemory);
        }

        // calculate offsets
        let base_offset = (NORMAL_PAGE_SIZE - total_payload_size) & !0xF;

        let sysv_offset = base_offset;
        let pkg_offset = sysv_offset + sysv_total_size;
        let handles_offset = pkg_offset + initpkg_size;
        let argv_offset = handles_offset + handles_array_size;
        let strings_offset = argv_offset + argv_array_size;

        // hhdm ptrs
        let hhdm_addr = phys_frame + *HHDMOFFSET;

        let pkg_hhdm_ptr = (hhdm_addr + pkg_offset) as *mut ProcessInitPackage;
        let handles_hhdm_ptr = (hhdm_addr + handles_offset) as *mut HandleGrant;
        let argv_hhdm_ptr = (hhdm_addr + argv_offset) as *mut *const u8;
        let strings_hhdm_ptr = (hhdm_addr + strings_offset) as *mut u8;
        let sysv_hhdm_ptr = (hhdm_addr + sysv_offset) as *mut u8;

        // virt addrs
        let base_vaddr = stack_vaddr + top_page_offset;
        let pkg_vaddr = base_vaddr + pkg_offset;
        let handles_vaddr = base_vaddr + handles_offset;
        let argv_vaddr = base_vaddr + argv_offset;
        let strings_vaddr = base_vaddr + strings_offset;
        let sysv_vaddr = base_vaddr + sysv_offset;

        unsafe {
            // copy raw strings buffer
            if strings_size > 0 {
                copy_nonoverlapping(args_buffer.as_ptr(), strings_hhdm_ptr, strings_size);
            }

            // build argv pointer array
            let mut current_string_vaddr = strings_vaddr;
            let mut arg_idx = 0;

            let mut i = 0;
            let mut start = 0;
            while i < strings_size {
                if args_buffer[i] == 0 {
                    write(argv_hhdm_ptr.add(arg_idx), current_string_vaddr as *const u8);
                    arg_idx += 1;

                    let str_len = (i - start) + 1;
                    current_string_vaddr += str_len;
                    start = i + 1;
                }
                i += 1
            }

            // null terminate argv array
            write(argv_hhdm_ptr.add(argc), null());

            // copy handle grants
            core::ptr::copy_nonoverlapping(extra_handles.as_ptr(), handles_hhdm_ptr, extra_handles.len());

            // populate and write init package
            initpkg.extra_handles_ptr = handles_vaddr as *const HandleGrant;
            initpkg.argc = argc;
            initpkg.argv = argv_vaddr as *const *const u8;
            core::ptr::write(pkg_hhdm_ptr, initpkg);

            // build standard sysv abi stack
            let mut ptr = sysv_hhdm_ptr as *mut usize;

            core::ptr::write(ptr, argc); // argc
            ptr = ptr.add(1);

            let mut current_string_vaddr = strings_vaddr;
            let mut start = 0;
            for i in 0..strings_size {
                if args_buffer[i] == 0 {
                    core::ptr::write(ptr, current_string_vaddr);
                    ptr = ptr.add(1);

                    let str_len = (i - start) + 1;
                    current_string_vaddr += str_len;
                    start = i + 1;
                }
            }
            core::ptr::write(ptr, 0); // null terminate argv
            ptr = ptr.add(1);

            core::ptr::write(ptr, 0); // null terminate envp (no env vars)
            ptr = ptr.add(1);

            let aux_ptr = ptr as *mut AuxEntry;
            core::ptr::write(aux_ptr.add(0), AuxEntry { a_type: 3, a_val: phdr_addr }); // AT_PHDR
            core::ptr::write(aux_ptr.add(1), AuxEntry { a_type: 4, a_val: phnum }); // AT_PHNUM
            core::ptr::write(aux_ptr.add(2), AuxEntry { a_type: 5, a_val: 56 }); // AT_PHENT (size of Elf64_Phdr)
            core::ptr::write(aux_ptr.add(3), AuxEntry { a_type: 6, a_val: 4096 }); // AT_PAGESZ
            core::ptr::write(aux_ptr.add(4), AuxEntry { a_type: 9, a_val: entry_point }); // AT_ENTRY
            core::ptr::write(aux_ptr.add(5), AuxEntry { a_type: 0, a_val: 0 }); // AT_NULL
        }

        let safe_stack_top = sysv_vaddr;
        Ok((pkg_vaddr, safe_stack_top))
    }
}
