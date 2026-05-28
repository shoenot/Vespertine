use core::fmt;

use vespertine_abi::{FileOp, HandleID, Invocation};

use crate::{get_init_pkg, syscall::sys_invoke};

pub struct SinkWriter {
    buf: [u8; 1024],
    pos: usize,
}

impl SinkWriter {
    pub const fn new() -> Self {
        Self { buf: [0u8; 1024], pos: 0 }
    }

    fn flush(&mut self) {
        if self.pos == 0 {
            return;
        }

        let pkg = get_init_pkg();
        let handle = if pkg.is_null() { HandleID(3) } else {
            unsafe { (*pkg).sink_handle }
        };

        let op = FileOp::Write { offset: 0, buffer_ptr: self.buf.as_ptr() as *mut u8, len: self.pos };
        let _ = sys_invoke(handle, &Invocation::File(op));
        self.pos = 0;
    }
}

impl fmt::Write for SinkWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut bytes = s.as_bytes();
        while !bytes.is_empty() {
            let space = self.buf.len() - self.pos;
            if space == 0 {
                self.flush();
                continue;
            }

            let chunk_size = core::cmp::min(bytes.len(), space);
            self.buf[self.pos..self.pos + chunk_size].copy_from_slice(&bytes[..chunk_size]);
            self.pos += chunk_size;
            bytes = &bytes[chunk_size..];
        }
        Ok(())
    }
}

impl Drop for SinkWriter {
	fn drop(&mut self) {
            self.flush();
	}
}

#[macro_export]
macro_rules! print {
($($arg:tt)*) => {
        {
            let mut writer = $crate::sink::SinkWriter::new();
    let _ = core::fmt::write(
                &mut writer,
                core::format_args!($($arg)*)
    );
        }
};
}

#[macro_export]
macro_rules! println {
($($arg:tt)*) => { $crate::print!("{}\n", format_args!($($arg)*)) };
}
