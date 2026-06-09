#![no_std]
#![no_main]
extern crate alloc;
use vespertine_abi::ProcessInitPackage;
use vespertine_rt::print;
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_rt::syscall::sys_read;
use vespertine_std::Error;
use vespertine_std::env;
use vespertine_std::term::set_raw_mode;
use vespertine_std::term::unset_raw_mode;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] tctest error: {:?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    set_raw_mode().expect("Error setting raw mode");
    let mut buf = [0u8; 1];
    loop {
        match sys_read(env::source(), buf.as_mut_ptr(), 1, 0) {
            Ok(n) if n > 0 => {
                let c = buf[0];
                // Exit on Ctrl-C (0x03) or 'q' for convenience
                if c == b'q' || c == 0x03 {
                    unset_raw_mode()?;
                    return Ok(());
                }
                // Print the hex code and the character if printable
                if c.is_ascii_graphic() || c == b' ' {
                    print!("'{c}' (0x{c:02x})  ");
                } else {
                    print!("??? (0x{c:02x})  ");
                }
            }
            Ok(_) => {} // No data
            Err(e) => return Err(e.into()),
        }
    }
}
