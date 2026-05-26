use core::fmt;

use vespertine_abi::{FileOp, HandleID, Invocation};

use crate::{get_init_pkg, syscall::sys_invoke};

pub struct SinkWriter;

impl fmt::Write for SinkWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let pkg_ptr = get_init_pkg();
        let handle = if pkg_ptr.is_null() { HandleID(3) } else {
            unsafe { (*pkg_ptr).sink_handle }
        };

        let op = Invocation::File(
            FileOp::Write { 
                offset: 0, 
                buffer_ptr: s.as_ptr() as *mut u8, 
                len: s.len() 
            }
        );

        let _ = sys_invoke(handle, &op);
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        { let _ = core::fmt::write(
            &mut $crate::sink::SinkWriter,
            core::format_args!($($arg)*)
        ); }
    };
}


#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => { $crate::print!("{}\n", format_args!($($arg)*)) };
}
