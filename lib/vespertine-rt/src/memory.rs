
use vespertine_abi::{HandleID, Invocation, MemManOp, MemPoolOp, VmoOp, ProcOp};
use vespertine_common::slab::PageProvider;
use crate::{rt_print, syscall::{SysError, sys_close, sys_invoke, sys_lookup}};

pub fn get_memory_manager() -> Result<HandleID, SysError> {
    let root = HandleID(0);
    let objects_dir = sys_lookup(root, "Objects")?;
    let mem_man = sys_lookup(objects_dir, "MemoryManager")?;
    Ok(mem_man)
}

pub fn create_private_pool(mem_man: HandleID) -> Result<HandleID, SysError> {
    let op = Invocation::MemoryManager(MemManOp::CreatePool { limit: 0 });
    let pool_idx = sys_invoke(mem_man, &op)?;
    Ok(HandleID(pool_idx))
}

pub struct UserPageProvider {
    pub mem_pool_handle: HandleID,
}

impl PageProvider for UserPageProvider {
    fn allocate_pages(&self, size: usize) -> *mut u8 {
        let alloc_op = Invocation::MemPool(MemPoolOp::AllocateVmo { size });
        let vmo_idx = sys_invoke(self.mem_pool_handle, &alloc_op)
            .expect("Out of memory: MemPool exhausted");
        let vmo_handle = HandleID(vmo_idx);

        let map_op = Invocation::Vmo(VmoOp::MapIntoProc { 
            vaddr: 0, 
            len: size, 
            vm_flags: 5
        });

        let mapped_addr = sys_invoke(vmo_handle, &map_op)
            .expect("Out of memory: Out of virtual memory");

        let _ = sys_close(vmo_handle);

        mapped_addr as *mut u8
    }

    fn free_pages(&self, ptr: *mut u8, size: usize) {
        if ptr.is_null() || size == 0 {
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
