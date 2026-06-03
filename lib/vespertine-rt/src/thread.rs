extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use vespertine_abi::{Invocation, ProcOp};

use crate::{
    get_init_pkg,
    memory::{create_private_pool, get_memory_manager},
    syscall::{SysError, sys_invoke, sys_mmap},
};

const DEFAULT_STACK_SIZE: usize = 4096 * 16;
const VM_FLAGS_STACK: usize = 0b101;

struct ThreadArgs {
    func: Box<dyn FnOnce() + Send + 'static>,
    done: Arc<AtomicBool>,
}

extern "sysv64" fn thread_trampoline(arg: usize) -> ! {
    let args: Box<ThreadArgs> = unsafe { Box::from_raw(arg as *mut ThreadArgs) };
    (args.func)();
    args.done.store(true, Ordering::Release);

    unsafe {
        asm!(
            "mov rax, 2",
            "syscall",
            options(noreturn),
        );
    }
}

// Public side 

/// Handle to spawned thread. supports polling for completion.
pub struct JoinHandle {
    done: Arc<AtomicBool>,
}

impl JoinHandle {
    #[inline]
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}

pub fn spawn<F>(f: F) -> Result<JoinHandle, SysError> 
where 
    F: FnOnce() + Send + 'static, 
{
    // allocate stack
    let mem_man = get_memory_manager()?;
    let pool = create_private_pool(mem_man)?;
    let stack_base = sys_mmap(pool, DEFAULT_STACK_SIZE, 0, VM_FLAGS_STACK)?;
    let stack_top = stack_base + DEFAULT_STACK_SIZE;

    // box closure + done flag and leak it 
    let done = Arc::new(AtomicBool::new(false));
    let args = Box::new(ThreadArgs {
        func: Box::new(f),
        done: Arc::clone(&done),
    });
    let arg_ptr = Box::into_raw(args) as usize;

    let self_handle = {
        let pkg = get_init_pkg();
        if pkg.is_null() {
            drop(unsafe { Box::from_raw(arg_ptr as *mut ThreadArgs) });
            return Err(SysError::InvalidHandle);
        }
        unsafe { (*pkg).self_handle }
    };
    
    let op = Invocation::Proc(ProcOp::SpawnThread { 
        entry: thread_trampoline as *const () as usize,
        stack_top, 
        arg: arg_ptr,
        priority: 1, 
    });

    match sys_invoke(self_handle, &op) {
        Ok(_) => Ok(JoinHandle { done }),
        Err(e) => {
            drop(unsafe { Box::from_raw(arg_ptr as *mut ThreadArgs) });
            Err(e)
        }
    }
}
