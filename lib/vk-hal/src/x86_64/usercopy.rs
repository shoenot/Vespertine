use core::ptr::copy_nonoverlapping;

pub const KERNEL_BASE: usize = 0xFFFF_8000_0000_0000;

unsafe extern "sysv64" {
    #[link_name = "copy_from_user"]
    fn arch_copy_from_user(dst: *mut u8, src: *const u8, len: usize) -> bool;

    #[link_name = "copy_to_user"]
    fn arch_copy_to_user(dst: *mut u8, src: *const u8, len: usize) -> bool;
}

#[inline]
fn is_kernel_address(addr: usize) -> bool { addr >= KERNEL_BASE }

#[inline]
pub fn copy_from_user(dst: *mut u8, src: *const u8, len: usize) -> bool {
    unsafe { arch_copy_from_user(dst, src, len) }
}

#[inline]
pub fn copy_to_user(dst: *mut u8, src: *const u8, len: usize) -> bool {
    unsafe { arch_copy_to_user(dst, src, len) }
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
