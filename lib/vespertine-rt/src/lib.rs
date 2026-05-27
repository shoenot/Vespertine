#![no_std]
#![no_main]
pub mod syscall;
pub mod sink;
pub mod source;
mod memory;

use core::{alloc::{GlobalAlloc, Layout}, arch::asm, panic::PanicInfo, ptr::{null, null_mut}};
use vespertine_abi::{FileOp, HandleID, Invocation, ProcessInitPackage};
use vespertine_common::{lock::TicketLock, slab::SlabAllocator};

use crate::{memory::{UserPageProvider, create_private_pool, get_memory_manager}, syscall::sys_invoke};

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

static mut INITIAL_PACKAGE: *const ProcessInitPackage = null();

pub fn get_init_pkg() -> *const ProcessInitPackage {
    unsafe { INITIAL_PACKAGE }
}

#[unsafe(no_mangle)]
pub extern "sysv64" fn _start(initpkg_ptr: *const ProcessInitPackage) -> ! {
    unsafe {
        INITIAL_PACKAGE = initpkg_ptr;
    }

    init_heap();

    unsafe {
        main(initpkg_ptr);
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
    pub fn main(pkg: *const ProcessInitPackage);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("\n[PANIC] Userland panic: {}", info);
    loop {}
}
