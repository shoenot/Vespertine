use core::ffi::{CStr, c_char};

use vespertine_abi::{HandleGrant, HandleID, ProcessInitPackage};
use vespertine_rt::get_init_pkg;
use alloc::vec::Vec;
use alloc::string::String;

extern crate alloc;

fn pkg() -> &'static ProcessInitPackage {
    let ptr = get_init_pkg();
    if ptr.is_null() {
        panic!("Uninitialized Process Environment: cannot find ProcessInitPackage");
    }
    unsafe { &*ptr }
}

pub fn args() -> Vec<String> {
    let p = pkg();
    let mut ret = Vec::with_capacity(p.argc);

    for i in 0..p.argc {
        unsafe { 
            let arg_ptr = *p.argv.add(i);
            if !arg_ptr.is_null() {
                let c_str = CStr::from_ptr(arg_ptr as *const c_char);
                ret.push(c_str.to_string_lossy().into_owned());
            }
        }
    }
    ret
}

pub fn sink() -> HandleID {
    pkg().sink_handle
}

pub fn source() -> HandleID {
    pkg().source_handle
}

pub fn root() -> HandleID {
    pkg().root_handle
}

pub fn self_handle() -> HandleID {
    pkg().self_handle
}

pub fn extra_handles() -> &'static [HandleGrant] {
    pkg().ext()
}

pub fn find_tag(tag: usize) -> Option<&'static HandleGrant> {
    let grants = extra_handles();
    let ret = grants.iter()
        .find(|g| g.tag == tag);
    ret
}
