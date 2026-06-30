extern crate alloc;
use alloc::alloc::alloc;
use alloc::alloc::dealloc;
use core::alloc::{Layout, LayoutError};
use core::ptr::copy_nonoverlapping;
use core::sync::atomic::Ordering;

use crate::x86_64::cpu::BootAllocFn;
use crate::x86_64::fpu::{
    CLEAN_LEGACY_FPU_CXT,
    CLEAN_FPU_CXT,
    FPU_CXT_SIZE,
    LegacyXtCxt,
    USE_XSAVE,
    FpuError,
    gen_avx_dummy_fpu,
    use_xsave,
};

use crate::x86_64::cpu::{
    KERNEL_CS,
    KERNEL_SS,
    USER_CS,
    USER_SS,
};

#[derive(Debug)]
pub enum ContextError {
    Layout(LayoutError),
    AllocationFailed,
    ExtendedContext,
}

impl From<FpuError> for ContextError {
    fn from(_: FpuError) -> Self {
        Self::ExtendedContext
    }
}

impl From<LayoutError> for ContextError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

#[repr(C, align(16))]
pub struct ThreadContext {
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rbp: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,

    pub interrupt_number: u64,
    pub error_code: u64,

    pub instruction_pointer: u64,
    pub code_segment: u64,
    pub cpu_flags: u64,
    pub stack_pointer: u64,
    pub stack_segment: u64,
}

impl ThreadContext {
    pub fn zero_gp(&mut self) {
        self.rax = 0;
        self.rbx = 0;
        self.rcx = 0;
        self.rdx = 0;
        self.rsi = 0;
        self.rdi = 0;
        self.rbp = 0;
        self.r8 = 0;
        self.r9 = 0;
        self.r10 = 0;
        self.r11 = 0;
        self.r12 = 0;
        self.r13 = 0;
        self.r14 = 0;
        self.r15 = 0;
    }

    pub fn init(&mut self, entry_point: u64, stack_top: u64, arg: usize) {
        self.zero_gp();
        self.instruction_pointer = entry_point;
        self.stack_pointer = stack_top;
        self.code_segment = KERNEL_CS;
        self.stack_segment = KERNEL_SS;
        self.cpu_flags = 0x202; // IF set
        self.rdi = arg;
    }

    pub fn init_user(&mut self, entry_point: u64, stack_top: u64, arg: usize) {
        self.zero_gp();
        self.instruction_pointer = entry_point;
        self.stack_pointer = stack_top;
        self.code_segment = USER_CS;
        self.stack_segment = USER_SS;
        self.cpu_flags = 0x202; // IF set

        self.rdi = arg;
    }
}

#[repr(C)]
pub(crate) struct SwitchContext {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub rbp: usize,
    pub rbx: usize,
    pub rip: usize,
}

impl SwitchContext {
    pub(crate) fn init(&mut self, rip: usize) {
        self.r12 = 0;
        self.r13 = 0;
        self.r14 = 0;
        self.r15 = 0;
        self.rbx = 0;
        self.rbp = 0;
        self.rip = rip;
    }
}

#[repr(C)]
pub struct SyscallFrame {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rbp: usize,
    pub rdi: usize,
    pub rsi: usize,
    pub rdx: usize,
    pub rcx: usize,
    pub rbx: usize,
    pub rax: usize,
    pub user_rsp: usize,
}

fn init_stack_context(
    entry_point: usize,
    arg: usize,
    stack_base: usize,
    stack_size: usize,
    is_user: bool,
    user_stack_top: usize,
) -> usize {
    let stack_top = stack_base + stack_size;
    let context_addr = stack_top - size_of::<ThreadContext>();
    let context_addr = context_addr & !0xF;
    let context = unsafe { &mut *(context_addr as *mut ThreadContext) };

    if is_user {
        context.init_user(entry_point as u64, user_stack_top as u64, arg);
    } else {
        context.init(entry_point as u64, (stack_top - 8) as u64, arg);
    }

    let switch_addr = context_addr - size_of::<SwitchContext>();
    let switch_context = unsafe { &mut *(switch_addr as *mut SwitchContext) };

    unsafe extern "C" {
        fn thread_entry_stub();
    }

    switch_context.init((thread_entry_stub as *const ()) as usize);

    switch_addr
}

pub fn init_thread_stack(
    entry_point: usize, arg: usize, stack_base: usize, stack_size: usize, is_user: bool,
    user_stack_top: usize,
) -> Result<(usize, *mut u8), ContextError> {
    let fpu_size = FPU_CXT_SIZE.load(Ordering::Relaxed);

    let fpu_ptr = if USE_XSAVE.load(Ordering::Relaxed) {
        gen_avx_dummy_fpu()?
    } else {
        let fpu_layout = Layout::from_size_align(fpu_size, 16)?;
        let fpu_ptr = unsafe { alloc(fpu_layout) as *mut u8 };

        if fpu_ptr.is_null() {
            return Err(ContextError::AllocationFailed);
        }

        let def = CLEAN_LEGACY_FPU_CXT.lock();
        let default_fpu_ref = def.as_ref().expect("Clean FPU not initialized");
        unsafe { copy_nonoverlapping(default_fpu_ref as *const LegacyXtCxt, fpu_ptr as *mut
        LegacyXtCxt, 1) };

        fpu_ptr
    };

    let switch_addr = init_stack_context(entry_point, arg, stack_base, stack_size, is_user,
    user_stack_top);

    Ok((switch_addr, fpu_ptr))
}

pub fn init_bootstrap_thread_stack(
    entry_point: usize,
    arg: usize,
    stack_base: usize,
    stack_size: usize,
    alloc: BootAllocFn,
) -> (usize, *mut u8) {
    let extended_context = allocate_bootstrap_extended_context(alloc);
    let switch_addr = init_stack_context(entry_point, arg, stack_base, stack_size, false, 0);

    (switch_addr, extended_context)
}

pub fn allocate_bootstrap_extended_context(alloc: BootAllocFn) -> *mut u8 {
    let fpu_ptr = if USE_XSAVE.load(Ordering::Relaxed) {
        let size = FPU_CXT_SIZE.load(Ordering::Relaxed);
        let fpu_ptr = alloc(size, 64) as *mut u8;
        let def = CLEAN_FPU_CXT.load(Ordering::Relaxed);
        unsafe { copy_nonoverlapping(def, fpu_ptr, size) };
        fpu_ptr
    } else {
        let fpu_size = FPU_CXT_SIZE.load(Ordering::Relaxed);
        alloc(fpu_size, 16) as *mut u8
    };
    fpu_ptr
}

pub unsafe fn deallocate_extended_context(extended_context: *mut u8) -> Result<(), ContextError> {
    if extended_context.is_null() { return Ok(()); }
    let size = FPU_CXT_SIZE.load(Ordering::Relaxed);
    let align = if USE_XSAVE.load(Ordering::Relaxed) { 64 } else { 16 };
    let layout = Layout::from_size_align(size, align)?;
    unsafe { dealloc(extended_context, layout); }
    Ok(())
}


unsafe extern "sysv64" {
    fn switch_threads_avx(
        old_stack_ptr: *mut usize, 
        new_stack_ptr: usize, 
        old_extended_context: *mut u8, 
        new_extended_context: *const u8,
    );

    fn switch_threads_legacy(
        old_stack_ptr: *mut usize, 
        new_stack_ptr: usize, 
        old_extended_context: *mut u8, 
        new_extended_context: *const u8,
    );
}

pub unsafe fn switch_threads(
    old_stack_ptr: *mut usize,
    new_stack_ptr: usize,
    old_extended_context: *mut u8,
    new_extended_context: *const u8,
) {
    unsafe {
        if use_xsave() {
            switch_threads_avx(old_stack_ptr, new_stack_ptr, old_extended_context, new_extended_context);
        } else {
            switch_threads_legacy(old_stack_ptr, new_stack_ptr, old_extended_context, new_extended_context);
        }
    }
}

pub unsafe fn switch_from_bootstrap(new_stack_ptr: usize, new_extended_context: *const u8) -> Result<(), ContextError> {
    let mut dummy_stack_ptr = 0usize;

    if use_xsave() {
        let dummy_fpu_ptr = gen_avx_dummy_fpu()?;
        unsafe {
            switch_threads_avx(
                &mut dummy_stack_ptr as *mut usize,
                new_stack_ptr,
                dummy_fpu_ptr,
                new_extended_context,
            );
        }
    } else {
        let mut dummy_fpu = LegacyXtCxt::new();
        unsafe {
            let dummy_fpu_ptr = &mut dummy_fpu as *mut LegacyXtCxt as *mut u8;
            switch_threads_legacy(
                &mut dummy_stack_ptr as *mut usize,
                new_stack_ptr,
                dummy_fpu_ptr,
                new_extended_context,
            );
        }
    }
    Ok(())
}
