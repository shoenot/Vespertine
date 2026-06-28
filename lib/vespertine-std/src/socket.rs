use core::slice;
extern crate alloc;
use alloc::vec::Vec;

use vespertine_abi::protocol::{
    PacketFlags,
    PacketHeader,
    VESPER_MAGIC,
};
use vespertine_abi::tag::CAP_SOCKFAC;
use vespertine_abi::{
    AccessRights,
    HandleID,
    Signal,
};
use vespertine_rt::once::OnceCell;
use vespertine_rt::syscall::{
    sys_close,
    sys_create_socket,
    sys_read,
    sys_set_nb,
    sys_wait,
    sys_write,
};

use crate::Error;
use crate::broker::Broker;
use crate::fs::{
    Path,
    resolve,
};
use crate::io::{
    Read,
    Write,
};

pub const MAX_PACKET_PAYLOAD: usize = 64 * 1024;

pub struct SocketFactory {
    handle: HandleID,
}

pub struct PacketFrame {
    pub header: PacketHeader,
    pub payload: Vec<u8>,
}

impl SocketFactory {
    pub fn request() -> Result<Self, Error> {
        let broker_handle = resolve(&Path::new("/System/Services/Socket"), AccessRights::READ).map_err(Error::from)?;
        let broker = Broker::from_handle(broker_handle);
        let handle = broker.request(CAP_SOCKFAC, AccessRights::CREATE)?;
        Ok(Self { handle })
    }

    pub fn new_pair(&self) -> Result<(Socket, Socket), Error> {
        let (h1, h2) = sys_create_socket(self.handle).map_err(Error::from)?;
        Ok((Socket { handle: Some(h1), owned: true }, Socket { handle: Some(h2), owned: true }))
    }
}

impl Drop for SocketFactory {
    fn drop(&mut self) { let _ = sys_close(self.handle); }
}

static SOCKET_FACTORY: OnceCell<SocketFactory> = OnceCell::new();

fn socket_factory() -> Result<&'static SocketFactory, Error> { SOCKET_FACTORY.get_or_try_init(SocketFactory::request) }

pub struct Socket {
    handle: Option<HandleID>,
    owned: bool,
}

impl Socket {
    pub fn new_pair() -> Result<(Socket, Socket), Error> { socket_factory()?.new_pair() }

    pub fn from_handle(handle: HandleID) -> Self { Socket { handle: Some(handle), owned: true } }

    pub fn borrow_handle(handle: HandleID) -> Self { Socket { handle: Some(handle), owned: false } }

    pub fn handle(&self) -> HandleID { self.handle.expect("Socket already closed") }

    pub fn set_nonblocking(&self, nb: bool) -> Result<(), Error> {
        sys_set_nb(self.handle(), nb).map_err(Error::from)?;
        Ok(())
    }

    pub fn wait(&self, signal: Signal) -> Result<(), Error> {
        sys_wait(self.handle(), signal).map_err(Error::from)?;
        Ok(())
    }

    pub fn send_packet<P: PacketPayload + ?Sized>(&self, packet_type: u32, payload: &P) -> Result<(), Error> {
        let header = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type,
            payload_len: payload.payload_len() as u32,
            reserved: 0,
        };

        let header_bytes = unsafe { slice::from_raw_parts(&header as *const _ as *const u8, size_of::<PacketHeader>()) };

        self.write_all(header_bytes)?;
        payload.write_payload(self)?;
        Ok(())
    }

    pub fn recv_packet<P: DecodePacketPayload>(&self) -> Result<(PacketHeader, P), Error> {
        let mut header = PacketHeader::default();

        let header_bytes = unsafe { slice::from_raw_parts_mut(&mut header as *mut _ as *mut u8, size_of::<PacketHeader>()) };

        self.read_exact(header_bytes)?;

        if header.magic != VESPER_MAGIC {
            return Err(Error::invalid_argument("malformed packet (invalid magic number)".into()));
        }

        let mut payload = Vec::new();
        payload.resize(header.payload_len as usize, 0);
        self.read_exact(&mut payload)?;

        let decoded = P::decode_packet_payload(&payload)?;
        Ok((header, decoded))
    }

    pub fn send_frame(&self, packet_type: u32, payload: &[u8]) -> Result<(), Error> {
        let payload_len = u32::try_from(payload.len()).map_err(|_| Error::invalid_argument("packet payload is too large".into()))?;

        if payload.len() > MAX_PACKET_PAYLOAD {
            return Err(Error::invalid_argument("packet payload exceeds limit".into()));
        }

        let header =
            PacketHeader { magic: VESPER_MAGIC, version: 1, packet_flags: PacketFlags::IS_BUFFER, packet_type, payload_len, reserved: 0 };

        let header_bytes = unsafe { slice::from_raw_parts(&header as *const PacketHeader as *const u8, size_of::<PacketHeader>()) };

        self.write_all(header_bytes)?;
        self.write_all(payload)
    }

    pub fn recv_frame(&self) -> Result<PacketFrame, Error> {
        let mut header = PacketHeader::default();

        let header_bytes = unsafe { slice::from_raw_parts_mut(&mut header as *mut PacketHeader as *mut u8, size_of::<PacketHeader>()) };

        self.read_exact(header_bytes)?;

        if header.magic != VESPER_MAGIC {
            return Err(Error::invalid_argument("invalid packet magic".into()));
        }

        if header.version != 1 {
            return Err(Error::invalid_argument("unsupported packet version".into()));
        }

        let payload_len = header.payload_len as usize;
        if payload_len > MAX_PACKET_PAYLOAD {
            return Err(Error::invalid_argument("packet payload exceeds limit".into()));
        }

        let mut payload = Vec::new();
        payload.resize(payload_len, 0);
        self.read_exact(&mut payload)?;

        Ok(PacketFrame { header, payload })
    }

    pub fn read_timeout(&self, buf: &mut [u8], timeout_ds: usize) -> Result<usize, Error> {
        sys_read(self.handle(), buf.as_mut_ptr(), buf.len(), timeout_ds).map_err(Error::from)
    }

    pub fn read_exact_timeout(&self, mut buf: &mut [u8], timeout_ds: usize) -> Result<(), Error> {
        while !buf.is_empty() {
            match self.read_timeout(buf, timeout_ds) {
                Ok(0) => {
                    return Err(Error::end_of_stream("unexpected end of stream during timed read".into()));
                },
                Ok(n) => buf = &mut buf[n..],
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub fn recv_frame_timeout(&self, timeout_ds: usize) -> Result<PacketFrame, Error> {
        let mut header = PacketHeader::default();
    
        let header_bytes = unsafe {
            slice::from_raw_parts_mut(&mut header as *mut PacketHeader as *mut u8, size_of::<PacketHeader>())
        };
    
        self.read_exact_timeout(header_bytes, timeout_ds)?;
    
        if header.magic != VESPER_MAGIC {
            return Err(Error::invalid_argument("invalid packet magic".into()));
        }
    
        if header.version != 1 {
            return Err(Error::invalid_argument("unsupported packet version".into()));
        }
    
        let payload_len = header.payload_len as usize;
        if payload_len > MAX_PACKET_PAYLOAD {
            return Err(Error::invalid_argument("packet payload exceeds limit".into()));
        }
    
        let mut payload = Vec::new();
        payload.resize(payload_len, 0);
        self.read_exact_timeout(&mut payload, timeout_ds)?;
    
        Ok(PacketFrame { header, payload })
    }

    pub fn close(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = sys_close(handle);
        }
    }
}

impl Read for Socket {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> { sys_read(self.handle(), buf.as_mut_ptr(), buf.len(), 0).map_err(Error::from) }
}

impl Write for Socket {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> { sys_write(self.handle(), buf.as_ptr(), buf.len(), 0).map_err(Error::from) }
}

impl Drop for Socket {
    fn drop(&mut self) {
        if self.owned {
            if let Some(handle) = self.handle.take() {
                let _ = sys_close(handle);
            }
        }
    }
}

pub trait PacketPayload {
    fn payload_len(&self) -> usize;
    fn write_payload(&self, socket: &Socket) -> Result<(), Error>;
}

impl<T: Copy> PacketPayload for T {
    fn payload_len(&self) -> usize { size_of::<T>() }

    fn write_payload(&self, socket: &Socket) -> Result<(), Error> {
        let bytes = unsafe { core::slice::from_raw_parts(self as *const T as *const u8, size_of::<T>()) };
        socket.write_all(bytes)
    }
}

impl PacketPayload for str {
    fn payload_len(&self) -> usize { self.as_bytes().len() }

    fn write_payload(&self, socket: &Socket) -> Result<(), Error> { socket.write_all(self.as_bytes()) }
}

pub trait DecodePacketPayload: Sized {
    fn decode_packet_payload(payload: &[u8]) -> Result<Self, Error>;
}

impl<T: Copy> DecodePacketPayload for T {
    fn decode_packet_payload(payload: &[u8]) -> Result<Self, Error> {
        if payload.len() != size_of::<T>() {
            return Err(Error::invalid_argument("packet payload size mismatch".into()));
        }

        let mut value = unsafe { core::mem::zeroed::<T>() };
        let value_bytes = unsafe { core::slice::from_raw_parts_mut(&mut value as *mut T as *mut u8, size_of::<T>()) };

        value_bytes.copy_from_slice(payload);
        Ok(value)
    }
}
