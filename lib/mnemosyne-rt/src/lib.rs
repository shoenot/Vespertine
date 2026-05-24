#![no_std]
#![no_main]
pub mod syscall;
mod memory;

use core::{alloc::{GlobalAlloc, Layout}, arch::asm, panic::PanicInfo, ptr::null_mut};
use mnemosyne_abi::{HandleID, Invocation, FileOp};
use mnemosyne_common::{lock::TicketLock, slab::SlabAllocator};

use crate::memory::{UserPageProvider, create_private_pool, get_memory_manager};

pub struct GlobalUserAlloc {
    inner: TicketLock<Option<SlabAllocator<UserPageProvider>>>,
}

unsafe impl Send for GlobalUserAlloc {}
unsafe impl Sync for GlobalUserAlloc {}

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

    main(root_handle, self_handle, console_handle);

    unsafe {
        asm!(
            "mov rax, 2",           // syscall 2 (terminate)
            "syscall",
            options(noreturn)
        );
    }
}

fn main(root_handle: HandleID, self_handle: HandleID, console_handle:HandleID) {
    let msg = "Hello from Mnemosyne Userspace!\n";
    let write_op = Invocation::File(FileOp::Write { 
        offset: 0, 
        buffer_ptr: msg.as_ptr() as *mut u8, 
        len: msg.len(),
    });

    unsafe {
        asm!(
            "mov rax, 0",           // syscall 0 (invoke)
            "syscall",
            in("rdi") console_handle.0,
            in("rsi") &write_op as *const _ as usize,
        );
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
