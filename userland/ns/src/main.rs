#![no_std]
#![no_main]
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::tag::*;
use vespertine_std::env;



#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    let pm = env::find_tag(TAG_SYS_PROCMAN).map(|g| g.id);
    let sf = env::find_tag(TAG_SYS_SOCKFAC).map(|g| g.id);

    let args = env::args();
    
}
