use vespertine_abi::{
    FileOp,
    HandleID,
    Invocation,
};

use crate::syscall::sys_invoke;

pub fn read_line(buf: &mut [u8]) -> usize {
    let mut total = 0;
    loop {
        let op = Invocation::File(FileOp::Read { offset: 0, buffer_ptr: buf[total..].as_mut_ptr() as usize, len: 1 });
        match sys_invoke(HandleID(2), &op) {
            Ok(n) if n > 0 => {
                if buf[total] == b'\x08' {
                    if total > 0 {
                        total -= 1;
                    }
                } else {
                    total += n;
                    if buf[total - 1] == b'\n' || total >= buf.len() {
                        return total;
                    };
                }
            }
            _ => return total,
        }
    }
}
