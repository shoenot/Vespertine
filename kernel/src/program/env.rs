use alloc::sync::Arc;
use core::ptr::{
    self,
    copy_nonoverlapping,
};

use vespertine_abi::{
    AT_VESPERTINE_INITPKG,
    CapabilityGrant,
    ProcessInitPackage,
};
use vespertine_common::slab::NORMAL_PAGE_SIZE;

use crate::object::invoke::InvocationError;
use crate::memory::DIRECT_MAP_OFFSET;
use crate::memory::vmo::{
    PagedBackingStore,
    Vmo,
};

const AUX_ENTRY_COUNT: usize = 8;

pub struct ProcessEnvironment;

#[repr(C)]
struct AuxEntry {
    a_type: usize,
    a_val: usize,
}

impl ProcessEnvironment {
    pub fn inject(
        stack_vmo: &Arc<Vmo>, stack_vaddr: usize, stack_size: usize, capabilities: &[CapabilityGrant], args_buffer: &[u8], argc: usize,
        mut initpkg: ProcessInitPackage, entry_point: usize, phdr_addr: usize, phnum: usize, base_addr: usize,
    ) -> Result<(usize, usize), InvocationError> {
        let top_page_offset = stack_size - NORMAL_PAGE_SIZE;
        let phys_frame = stack_vmo.request_page(top_page_offset).map_err(|_| InvocationError::OutOfMemory)?;

        // calculate sizes
        let initpkg_size = size_of::<ProcessInitPackage>();
        let capabilities_array_size = capabilities.len() * size_of::<CapabilityGrant>();
        let _argv_array_size = (argc + 1) * size_of::<*const u8>();
        let strings_size = args_buffer.len();

        // System V stack structure: argc (usize) + argv (pointers) + null + envp (pointers, none) + null + 7 aux entries
        let sysv_total_size = (1 + (argc + 1) + 1) * size_of::<usize>() + AUX_ENTRY_COUNT * size_of::<AuxEntry>();

        // Pack top-down: sysv (lowest) -> pkg -> capabilities -> strings (highest)
        // This ensures RSP (at sysv) has the entire payload ABOVE it.
        let total_payload_size = sysv_total_size + 16 + initpkg_size + 16 + capabilities_array_size + 16 + strings_size;
        if total_payload_size > NORMAL_PAGE_SIZE {
            return Err(InvocationError::OutOfMemory);
        }

        let base_offset = (NORMAL_PAGE_SIZE - total_payload_size) & !0xF;

        let sysv_offset = base_offset;
        let pkg_offset = (sysv_offset + sysv_total_size + 15) & !0xF;
        let capabilities_offset = (pkg_offset + initpkg_size + 15) & !0xF;
        let strings_offset = (capabilities_offset + capabilities_array_size + 15) & !0xF;

        // hhdm ptrs
        let hhdm_addr = phys_frame + *DIRECT_MAP_OFFSET;

        let strings_hhdm_ptr = (hhdm_addr + strings_offset) as *mut u8;
        let capabilities_hhdm_ptr = (hhdm_addr + capabilities_offset) as *mut CapabilityGrant;
        let pkg_hhdm_ptr = (hhdm_addr + pkg_offset) as *mut ProcessInitPackage;
        let sysv_hhdm_ptr = (hhdm_addr + sysv_offset) as *mut u8;

        // virt addrs
        let base_vaddr = stack_vaddr + top_page_offset;
        let strings_vaddr = base_vaddr + strings_offset;
        let capabilities_vaddr = base_vaddr + capabilities_offset;
        let pkg_vaddr = base_vaddr + pkg_offset;
        let sysv_vaddr = base_vaddr + sysv_offset;

        unsafe {
            // copy raw strings buffer
            if strings_size > 0 {
                copy_nonoverlapping(args_buffer.as_ptr(), strings_hhdm_ptr, strings_size);
            }

            // build capability grants
            copy_nonoverlapping(capabilities.as_ptr(), capabilities_hhdm_ptr, capabilities.len());

            // build argv pointers manually on the stack side
            let mut sysv_ptr = sysv_hhdm_ptr as *mut usize;

            ptr::write(sysv_ptr, argc);
            sysv_ptr = sysv_ptr.add(1);

            let mut current_string_vaddr = strings_vaddr;
            let mut start = 0;
            for i in 0..strings_size {
                if args_buffer[i] == 0 {
                    ptr::write(sysv_ptr, current_string_vaddr);
                    sysv_ptr = sysv_ptr.add(1);

                    let str_len = (i - start) + 1;
                    current_string_vaddr += str_len;
                    start = i + 1;
                }
            }
            ptr::write(sysv_ptr, 0); // null terminate argv
            sysv_ptr = sysv_ptr.add(1);

            ptr::write(sysv_ptr, 0); // null terminate envp (no env vars)
            sysv_ptr = sysv_ptr.add(1);

            // build aux vector
            let aux_ptr = sysv_ptr as *mut AuxEntry;
            ptr::write(aux_ptr.add(0), AuxEntry { a_type: 3, a_val: phdr_addr }); // AT_PHDR
            ptr::write(aux_ptr.add(1), AuxEntry { a_type: 4, a_val: 56 }); // AT_PHENT
            ptr::write(aux_ptr.add(2), AuxEntry { a_type: 5, a_val: phnum }); // AT_PHNUM
            ptr::write(aux_ptr.add(3), AuxEntry { a_type: 6, a_val: 4096 }); // AT_PAGESZ
            ptr::write(aux_ptr.add(4), AuxEntry { a_type: 7, a_val: base_addr }); // AT_BASE
            ptr::write(aux_ptr.add(5), AuxEntry { a_type: 9, a_val: entry_point }); // AT_ENTRY
            ptr::write(aux_ptr.add(6), AuxEntry { a_type: AT_VESPERTINE_INITPKG, a_val: pkg_vaddr }); // AT_NULL
            ptr::write(aux_ptr.add(7), AuxEntry { a_type: 0, a_val: 0 });

            // Write init package
            initpkg.capabilities_ptr = capabilities_vaddr as *const CapabilityGrant;
            initpkg.argc = argc;
            initpkg.argv = (sysv_vaddr + size_of::<usize>()) as *const *const u8;
            ptr::write(pkg_hhdm_ptr, initpkg);
        }

        // The safe_stack_top is where the process starts (sysv_vaddr).
        // Since we are at the very base of our payload, stack growth (downwards)
        // will move AWAY from our data, keeping it perfectly safe.
        Ok((pkg_vaddr, sysv_vaddr))
    }
}
