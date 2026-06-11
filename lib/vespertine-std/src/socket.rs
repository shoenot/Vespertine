use core::{mem::zeroed, slice};

use vespertine_abi::{
    AccessRights, HandleID, Signal, protocol::{PacketFlags, PacketHeader, VESPER_MAGIC}, tag::{CAP_SOCKFAC}
};
use vespertine_rt::{once::OnceCell, syscall::{
    sys_close, sys_create_socket, sys_read, sys_set_nb, sys_wait, sys_write,
}};


pub struct SocketFactory {
    handle: HandleID,
}

impl SocketFactory {
    pub fn request() -> Result<Self, Error> {
        let broker_handle = walk_path("/System/Services/Socket", env::root()).map_err(Error::from)?;
        let broker = Broker::from_handle(broker_handle);
        let handle = broker.request(CAP_SOCKFAC, AccessRights::CREATE)?;
        Ok(Self { handle })
    }

    pub fn new_pair(&self) -> Result<(Socket, Socket), Error> {
        let (h1, h2) = sys_create_socket(self.handle).map_err(Error::from)?;
        Ok((Socket(Some(h1)), Socket(Some(h2))))
    }
}

impl Drop for SocketFactory {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}

use crate::{
    Error, ErrorKind, broker::Broker, env, fs::walk_path, io::{Read, Write}
};

static SOCKET_FACTORY: OnceCell<SocketFactory> = OnceCell::new();

fn socket_factory() -> Result<&'static SocketFactory, Error> {
    SOCKET_FACTORY.get_or_try_init(SocketFactory::request)
}

pub struct Socket(Option<HandleID>);

impl Socket {
    pub fn new_pair() -> Result<(Socket, Socket), Error> {
        socket_factory()?.new_pair()
    }

    pub fn from_handle(handle: HandleID) -> Self {
        Self(Some(handle))
    }

    pub fn handle(&self) -> HandleID {
        self.0.expect("Socket already closed")
    }

    pub fn set_nonblocking(&self, nb: bool) -> Result<(), Error> {
        sys_set_nb(self.handle(), nb).map_err(Error::from)?;
        Ok(())
    }

    pub fn wait(&self, signal: Signal) -> Result<(), Error> {
        sys_wait(self.handle(), signal).map_err(Error::from)?;
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

        let payload_bytes =
            unsafe { slice::from_raw_parts(payload as *const _ as *const u8, payload_size) };
        self.write_all(payload_bytes)?;

        Ok(())
    }

    pub fn recv_packet<T: Copy>(&self) -> Result<(PacketHeader, T), Error> {
        let mut header = PacketHeader::default();
        let header_size = size_of::<PacketHeader>();

        let header_bytes =
            unsafe { slice::from_raw_parts_mut(&mut header as *mut _ as *mut u8, header_size) };

        self.read_exact(header_bytes)?;

        if header.magic != VESPER_MAGIC {
            return Err(Error {
                kind: ErrorKind::InvalidArgument,
                message: "Invalid packet magic number".into(),
            });
        }

        if header.payload_len as usize != size_of::<T>() {
            return Err(Error {
                kind: ErrorKind::InvalidArgument,
                message: "Packet payload size mismatch".into(),
            });
        }

        let mut payload = unsafe { zeroed::<T>() };
        let payload_bytes =
            unsafe { slice::from_raw_parts_mut(&mut payload as *mut T as *mut u8, size_of::<T>()) };
        self.read_exact(payload_bytes)?;

        Ok((header, payload))
    }

    pub fn close(mut self) {
        if let Some(handle) = self.0.take() {
            let _ = sys_close(handle);
        }
    }
}

impl Read for Socket {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        sys_read(self.handle(), buf.as_mut_ptr(), buf.len(), 0).map_err(Error::from)
    }
}

impl Write for Socket {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        sys_write(self.handle(), buf.as_ptr(), buf.len(), 0).map_err(Error::from)
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = sys_close(handle);
        }
    }
}
