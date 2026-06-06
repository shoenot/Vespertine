use core::{mem::zeroed, slice};

use vespertine_abi::{HandleID, Signal, protocol::{PacketFlags, PacketHeader, VESPER_MAGIC}, tag::TAG_SYS_SOCKFAC};
use vespertine_rt::syscall::{
    sys_close, sys_create_socket, sys_read, sys_set_nb, sys_wait, sys_write,
};

use crate::{
    Error, ErrorKind, env,
    io::{Read, Write},
};

pub struct Socket {
    read_handle: Option<HandleID>,
    write_handle: Option<HandleID>,
}

impl Socket {
    pub fn new() -> Result<Self, Error> {
        let sf = env::find_tag(TAG_SYS_SOCKFAC)
            .expect("Socket Factory not found")
            .id;
        let (r, w) = sys_create_socket(sf).map_err(Error::from)?;
        Ok(Socket {
            read_handle: Some(r),
            write_handle: Some(w),
        })
    }

    pub fn from_read_handle(handle: HandleID) -> Self {
        Socket {
            read_handle: Some(handle),
            write_handle: None,
        }
    }

    pub fn from_write_handle(handle: HandleID) -> Self {
        Socket {
            read_handle: None,
            write_handle: Some(handle),
        }
    }

    pub fn read_handle(&self) -> Result<HandleID, Error> {
        self.read_handle.ok_or(Error {
            kind: ErrorKind::InvalidHandle,
            message: "No read handle!".into(),
        })
    }

    pub fn write_handle(&self) -> Result<HandleID, Error> {
        self.write_handle.ok_or(Error {
            kind: ErrorKind::InvalidHandle,
            message: "No write handle!".into(),
        })
    }

    pub fn close_write(&mut self) {
        if let Some(h) = self.write_handle.take() {
            let _ = sys_close(h);
        }
    }

    pub fn setnb(&self, nb: bool) -> Result<(), Error> {
        if let Some(r) = self.read_handle {
            sys_set_nb(r, nb).map_err(Error::from)?;
        }
        if let Some(w) = self.write_handle {
            sys_set_nb(w, nb).map_err(Error::from)?;
        }
        Ok(())
    }

    pub fn wait(&self, signal: Signal) -> Result<(), Error> {
        if signal.contains(Signal::READABLE) || signal.contains(Signal::PEER_CLOSED) {
            let handle = self.read_handle().map_err(|_| Error {
                kind: ErrorKind::AccessDenied,
                message: "Socket is write-only, cannot wait for read".into(),
            })?;
            sys_wait(handle, signal).map_err(Error::from)?;
        } else if signal.contains(Signal::WRITABLE) {
            let handle = self.write_handle().map_err(|_| Error {
                kind: ErrorKind::AccessDenied,
                message: "Socket is read-only, cannot wait for write".into(),
            })?;
            sys_wait(handle, signal).map_err(Error::from)?;
        }
        Ok(())
    }

    pub fn send_packet<T: Copy>(&self, packet_type: u32, payload: &T) -> Result<(), Error> {
        let payload_size = size_of::<T>();

        let header = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type,
            payload_len: payload_size as u32,
            reserved: 0,
        };

        let header_bytes = unsafe {
            slice::from_raw_parts(&header as *const _ as *const u8, size_of::<PacketHeader>())
        };
        self.write_all(header_bytes)?;

        let payload_bytes = unsafe {
            slice::from_raw_parts(payload as *const _ as *const u8, payload_size)
        };
        self.write_all(payload_bytes)?;

        Ok(())
    }

    pub fn recv_packet<T: Copy>(&self) -> Result<(PacketHeader, T), Error> {
        let mut header = PacketHeader::default();
        let header_size = size_of::<PacketHeader>();

        let header_bytes = unsafe {
            slice::from_raw_parts_mut(&mut header as *mut _ as *mut u8, header_size)
        };

        self.read_exact(header_bytes)?;

        if header.magic != VESPER_MAGIC {
            return Err(Error {
                kind: ErrorKind::InvalidArgument,
                message: "Invalid packet magic number".into(),
            })
        }

        if header.payload_len as usize != size_of::<T>() {
            return Err(Error {
                kind: ErrorKind::InvalidArgument,
                message: "Packet payload size mismatch".into(),
            })
        }

        let mut payload = unsafe { zeroed::<T>() };
        let payload_bytes = unsafe {
            slice::from_raw_parts_mut(&mut payload as *mut T as *mut u8, size_of::<T>())
        };
        self.read_exact(payload_bytes)?;

        Ok((header, payload))
    }
}

impl Read for Socket {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let handle = self.read_handle.ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "Socket is write-only".into(),
        })?;

        sys_read(handle, buf.as_mut_ptr(), buf.len(), 0).map_err(Error::from)
    }
}

impl Write for Socket {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        let handle = self.write_handle.ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "Socket is read-only".into(),
        })?;

        sys_write(handle, buf.as_ptr(), buf.len(), 0).map_err(Error::from)
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        if let Some(h) = self.read_handle {
            let _ = sys_close(h);
        }
        if let Some(h) = self.write_handle {
            let _ = sys_close(h);
        }
    }
}
