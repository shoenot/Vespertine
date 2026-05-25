#![no_std]
#![no_main]
pub mod syscall;
mod memory;

use core::{alloc::{GlobalAlloc, Layout}, arch::asm, panic::PanicInfo, ptr::null_mut};
use vespertine_abi::{HandleID, Invocation, FileOp};
use vespertine_common::{lock::TicketLock, slab::SlabAllocator};

use crate::memory::{UserPageProvider, create_private_pool, get_memory_manager};

pub struct GlobalUserAlloc {
    inner: TicketLock<Option<SlabAllocator<UserPageProvider>>>,
}

unsafe impl Send for GlobalUserAlloc {}
unsafe impl Sync for GlobalUserAlloc {}

pub fn rt_print(text: &str) {
    let op = vespertine_abi::Invocation::File(vespertine_abi::FileOp::Write {
        offset: 0,
        buffer_ptr: text.as_ptr() as *mut u8,
        len: text.len(),
    });
    let console = vespertine_abi::HandleID(2);
    let _ = crate::syscall::sys_invoke(console, &op);
}

unsafe impl GlobalAlloc for GlobalUserAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let lock = self.inner.lock();
        if let Some(ref allocator) = *lock {
            unsafe { allocator.alloc(layout) }
        } else {
            null_mut()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let lock = self.inner.lock();
        if let Some(ref allocator) = *lock {
            unsafe { allocator.dealloc(ptr, layout) };
        }
    }
}

#[global_allocator]
pub static ALLOCATOR: GlobalUserAlloc = GlobalUserAlloc { inner: TicketLock::new(None) };

pub fn init_heap() {
    let mem_man = get_memory_manager().expect("MemoryManger not found");
    let pool = create_private_pool(mem_man).expect("Failed to create MemPool");
    let provider = UserPageProvider { mem_pool_handle: pool };
    let allocator = SlabAllocator::new(provider);

    let mut lock = ALLOCATOR.inner.lock();
    *lock = Some(allocator);
}

#[unsafe(no_mangle)]
pub extern "sysv64" fn _start() -> ! {
    let root_handle = HandleID(0);
    let self_handle = HandleID(1);
    let console_handle = HandleID(2);

    init_heap();

    unsafe {
        main(root_handle, self_handle, console_handle);
    }

    unsafe {
        asm!(
            "mov rax, 2",           // syscall 2 (terminate)
            "syscall",
            options(noreturn)
        );
    }
}

unsafe extern "sysv64" {
    pub fn main(r: HandleID, s: HandleID, c: HandleID);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
