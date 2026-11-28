use core::ptr::copy_nonoverlapping;

use crate::interrupts::TrapFrame;

pub const KERNEL_BASE: usize = 0xFFFF_8000_0000_0000;
pub const USER_SPACE_LIMIT: usize = 0x0000_8000_0000_0000;

unsafe extern "sysv64" {
    #[link_name = "copy_from_user"]
    fn arch_copy_from_user(dst: *mut u8, src: *const u8, len: usize) -> bool;

    #[link_name = "copy_to_user"]
    fn arch_copy_to_user(dst: *mut u8, src: *const u8, len: usize) -> bool;

    fn arch_load_user_u32_nofault(addr: *const u32, out: *mut u32) -> bool;
}

unsafe extern "C" {
    static user_u32_nofault_fault: u8;
    static user_u32_nofault_recover: u8;
}

#[inline]
fn is_kernel_address(addr: usize) -> bool { addr >= KERNEL_BASE }

#[inline]
pub fn copy_from_user(dst: *mut u8, src: *const u8, len: usize) -> bool { unsafe { arch_copy_from_user(dst, src, len) } }

#[inline]
pub fn copy_to_user(dst: *mut u8, src: *const u8, len: usize) -> bool { unsafe { arch_copy_to_user(dst, src, len) } }

#[inline]
pub unsafe fn copy_mapped_bytes(src: *const u8, dst: *mut u8, count: usize) {
    unsafe {
        core::arch::asm!(
            "cld",
            "rep movsb",
            inout("rsi") src => _,
            inout("rdi") dst => _,
            inout("rcx") count => _,
            options(nostack),
        );
    }
}

pub fn safe_copy_from(dst: *mut u8, src: *const u8, len: usize) -> bool {
    if is_kernel_address(src as usize) {
        unsafe {
            copy_nonoverlapping(src, dst, len);
        }

        true
    } else {
        copy_from_user(dst, src, len)
    }
}

pub fn safe_copy_to(dst: *mut u8, src: *const u8, len: usize) -> bool {
    if is_kernel_address(dst as usize) {
        unsafe {
            copy_nonoverlapping(src, dst, len);
        }

        true
    } else {
        copy_to_user(dst, src, len)
    }
}

pub fn load_user_u32_nofault(addr: usize) -> Option<u32> {
    if addr == 0 || addr % 4 != 0 || addr.checked_add(3)? >= USER_SPACE_LIMIT { return None };

    let mut value = 0;
    unsafe { arch_load_user_u32_nofault(addr as *const u32, &mut value) }
        .then_some(value)
}

pub fn fixup_nofault_user_access(frame: &mut TrapFrame) -> bool {
    if frame.user_mode() || frame.instruction_pointer != core::ptr::addr_of!(user_u32_nofault_fault) as u64 {
        return false;
    }

    frame.instruction_pointer = core::ptr::addr_of!(user_u32_nofault_recover) as u64;
    true
}
