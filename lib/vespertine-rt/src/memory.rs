use core::{ptr::null_mut, slice, sync::atomic::AtomicUsize};

use vespertine_abi::{HandleID, Invocation, MemManOp, MemPoolOp, ProcOp, Signal, VmoOp, protocol::{MemoryRequest, ResourceResponse}, tag::TAG_SYS_RES_MAN};
use vespertine_common::slab::PageProvider;
use crate::{get_init_pkg, syscall::{SysError, sys_close, sys_invoke, sys_lookup, sys_read, sys_wait, sys_write}};

pub fn get_memory_manager() -> Result<HandleID, SysError> {
    let root = HandleID(0);
    let sys_dir = sys_lookup(root, "System")?;
    let srv_dir = sys_lookup(sys_dir, "Services")?;
    let mem_man = sys_lookup(srv_dir, "MemoryManager")?;
    Ok(mem_man)
}

pub fn create_private_pool(mem_man: HandleID) -> Result<HandleID, SysError> {
    let op = Invocation::MemoryManager(MemManOp::CreatePool { limit: 0 });
    let pool_idx = sys_invoke(mem_man, &op)?;
    Ok(HandleID(pool_idx))
}

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
                        vm_flags: 5
                    });

                    let mapped_addr = sys_invoke(vmo_handle, &map_op)
                        .expect("Out of memory: Out of virtual memory");
                    let _ = sys_close(vmo_handle);
                    return mapped_addr as *mut u8;
                },
                Err(SysError::PoolExhausted) => {
                    let pkg = get_init_pkg();
                    if pkg.is_null() { return null_mut(); }
                    let res_man = unsafe {
                        match (*pkg).ext().iter().find(|g| g.tag == TAG_SYS_RES_MAN) {
                            Some(r) => r,
                            None => return null_mut(),
                        }
                    }.id;
                    let mut req = MemoryRequest {
                        requested_bytes: size,
                        pool_handle: self.mem_pool_handle,
                    };
                    let req_ptr = &mut req as *mut _ as *mut u8;
                    let req_size = size_of::<MemoryRequest>();
                    sys_write(res_man, req_ptr, req_size, 0)
                        .expect("Could not request more memory");

                    sys_wait(res_man, Signal::READABLE)
                        .expect("Could not request more memory");

                    let mut res = ResourceResponse { status: 0 };
                    let res_ptr = &mut res as *mut _ as *mut u8;
                    let res_size = size_of::<ResourceResponse>();
                    sys_read(res_man, res_ptr, res_size, 0)
                        .expect("Could not request more memory");

                    if res.status == 0 {
                        continue;
                    } else {
                        return null_mut();
                    }
                },
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
