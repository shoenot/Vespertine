mod process;
pub use process::*;

pub mod procman;
pub mod thread_object;

pub fn current_process<'a>() -> Option<&'a Process> {
    let thread = crate::cpu::current_core_mut().scheduler.get_current_thread();

    if thread.is_null() {
        crate::KERNEL_PROCESS.get()
    } else {
        unsafe { Some(&(*thread).process) }
    }
}
