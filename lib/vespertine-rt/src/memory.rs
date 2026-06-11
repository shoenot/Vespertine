use core::{ptr::null_mut, sync::atomic::AtomicUsize};

use crate::syscall::{SysError, sys_close, sys_invoke};
use vespertine_abi::{
    HandleID, Invocation, MemPoolOp, ProcOp, VmoOp,
};
use vespertine_common::slab::PageProvider;

pub struct UserPageProvider {
    pub mem_pool_handle: HandleID,
    pub arena_start: AtomicUsize,
    pub arena_offset: AtomicUsize,
    pub arena_size: usize,
}

impl PageProvider for UserPageProvider {
    fn allocate_pages(&self, size: usize) -> *mut u8 {
        use core::sync::atomic::Ordering;

        //  fast path - attempt to allocate from arena
        let mut offset = self.arena_offset.load(Ordering::Relaxed);
        loop {
            if offset + size <= self.arena_size {
                match self.arena_offset.compare_exchange_weak(
                    offset,
                    offset + size,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let ptr = (self.arena_start.load(Ordering::SeqCst) + offset) as *mut u8;
                        return ptr;
                    }
                    Err(actual) => offset = actual,
                }
            } else {
                break;
            }
        }

        // if no fast path then request vmo from kernel
        loop {
            let alloc_op = Invocation::MemPool(MemPoolOp::AllocateVmo { size });
            let vmo_idx = sys_invoke(self.mem_pool_handle, &alloc_op);
            match vmo_idx {
                Ok(idx) => {
                    let vmo_handle = HandleID(idx);
                    let map_op = Invocation::Vmo(VmoOp::MapIntoProc {
                        vaddr: 0,
                        len: size,
                        vm_flags: 5,
                    });

                    let mapped_addr = sys_invoke(vmo_handle, &map_op)
                        .expect("Out of memory: Out of virtual memory");
                    let _ = sys_close(vmo_handle);
                    return mapped_addr as *mut u8;
                }
                Err(SysError::PoolExhausted) => {
                    let expansion = Invocation::MemPool(MemPoolOp::RequestExpansion {
                        additional_bytes: size,
                    });
                    if sys_invoke(self.mem_pool_handle, &expansion).is_ok() {
                        continue;
                    }
                    return null_mut();
                }
                Err(_) => return null_mut(),
            }
        }
    }

    fn free_pages(&self, ptr: *mut u8, size: usize) {
        use core::sync::atomic::Ordering;

        if ptr.is_null() || size == 0 {
            return;
        }

        // skip unmapping if the memory is part of the arena
        let start = self.arena_start.load(Ordering::SeqCst);
        let addr = ptr as usize;
        if addr >= start && addr < start + self.arena_size {
            return;
        }

        let self_handle = HandleID(1);
        let unmap_op = Invocation::Proc(ProcOp::Unmap {
            vaddr: ptr as usize,
            len: size,
        });

        let _ = sys_invoke(self_handle, &unmap_op).expect("Process munmap failed");
    }
}
