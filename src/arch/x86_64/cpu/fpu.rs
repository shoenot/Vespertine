use alloc::alloc::alloc;
use core::{
    alloc::Layout,
    arch::asm,
    ptr::{
        copy_nonoverlapping,
        null_mut,
    },
    sync::atomic::{
        AtomicBool,
        AtomicPtr,
        AtomicUsize,
        Ordering,
    },
};

use crate::{
    arch::x86_64::cpuid::{
        check_xsave_support,
        get_xsave_details,
    },
    kernel::{
        sync::TicketLock,
        thread::ThreadError,
    },
};

pub(crate) static CLEAN_FPU_CXT: AtomicPtr<u8> = AtomicPtr::new(null_mut() as *mut u8);
pub(crate) static CLEAN_LEGACY_FPU_CXT: TicketLock<Option<LegacyXtCxt>> = TicketLock::new(None);
pub(crate) static FPU_CXT_SIZE: AtomicUsize = AtomicUsize::new(0);
pub(crate) static USE_XSAVE: AtomicBool = AtomicBool::new(false);

#[repr(C, align(16))]
pub(crate) struct LegacyXtCxt {
    pub fcw: u16,
    pub fsw: u16,
    pub ftw: u16,
    pub fop: u16,
    pub f_rip: u64,
    pub f_rdp: u64,
    pub mxcsr: u32,
    pub mxcsr_mask: u32,
    pub mmx_regs: [[u8; 16]; 8],
    pub sse_regs: [[u8; 16]; 16],
    pub reserved: [u8; 96],
}

impl LegacyXtCxt {
    pub(crate) const fn new() -> Self {
        Self {
            fcw: 0,
            fsw: 0,
            ftw: 0,
            fop: 0,
            f_rip: 0,
            f_rdp: 0,
            mxcsr: 0,
            mxcsr_mask: 0,
            mmx_regs: [[0; 16]; 8],
            sse_regs: [[0; 16]; 16],
            reserved: [0; 96],
        }
    }

    pub(crate) unsafe fn init_default_state(&mut self) {
        unsafe {
            asm!("fninit",
                "fxsave64 [{}]",
                in(reg) self,
                options(nostack, preserves_flags));
        }
        self.mxcsr = 0x1F80;
    }
}

#[repr(C, align(64))]
pub(crate) struct XtCxtFixed {
    pub legacy: LegacyXtCxt,
    pub xsave_header: [u8; 64],
}

unsafe extern "sysv64" {
    fn init_xsave();
    fn init_clean_fpu_state(ptr: *mut u8);
    pub fn init_cr4();
}

fn init_default_fpu_avx_cxt() -> Option<()> {
    if check_xsave_support() {
        let (eax, ..) = get_xsave_details();
        if (eax & (0x7)) == 0x7 {
            USE_XSAVE.store(true, Ordering::Relaxed);
            unsafe {
                init_xsave();
                let (_, ebx, _) = get_xsave_details();
                FPU_CXT_SIZE.store(ebx, Ordering::Relaxed);
                let clean_fpu_cxt_layout = Layout::from_size_align(ebx, 64).ok()?;
                let clean_fpu_cxt = alloc(clean_fpu_cxt_layout) as *mut u8;
                init_clean_fpu_state(clean_fpu_cxt);
                CLEAN_FPU_CXT.store(clean_fpu_cxt, Ordering::Relaxed);
                return Some(());
            }
        }
    }
    None
}

fn init_default_fpu_legacy_cxt() {
    let mut clean_state = LegacyXtCxt::new();
    unsafe {
        clean_state.init_default_state();
    }
    let mut cln = CLEAN_LEGACY_FPU_CXT.lock();
    *cln = Some(clean_state);
}

pub(crate) fn init_default_fpu_cxt() {
    if let Some(_) = init_default_fpu_avx_cxt() {
        return;
    } else {
        init_default_fpu_legacy_cxt();
    }
}

pub(crate) fn gen_avx_dummy_fpu() -> Result<*mut u8, ThreadError> {
    unsafe {
        let size = FPU_CXT_SIZE.load(Ordering::Relaxed);
        let fpu_layout = Layout::from_size_align(size, 64)?;
        let fpu_ptr = alloc(fpu_layout) as *mut u8;
        let def = CLEAN_FPU_CXT.load(Ordering::Relaxed);
        copy_nonoverlapping(def, fpu_ptr, size);
        Ok(fpu_ptr)
    }
}
