use vespertine_abi::tag::CAP_LOGGER;
use vespertine_abi::{
    AccessRights, HandleID, LogLevel,
};
use vrt::mutex::Mutex;
use vrt::syscall::{
    sys_close,
    sys_read,
    sys_write,
};

use crate::broker::Broker;
use crate::fs::{
    Path,
    resolve,
};
use crate::socket::Socket;
use crate::{
    Error,
    Read,
    Write,
};

pub const MAX_LOG_MESSAGE_BYTES: usize = 2048;

pub struct SystemLog(Mutex<Option<Socket>>);

impl SystemLog {
    pub fn try_connect() -> Result<Self, Error> {
        let handle = resolve(&Path::new("/System/Services/Log"), AccessRights::READ)?;
        let broker = Broker::from_handle(handle);
        let writer = broker.request(CAP_LOGGER, AccessRights::WRITE)?;
        Ok(Self::from_handle(writer))
    }

    pub fn connect() -> Self { Self::try_connect().expect("Could not find logger") }

    pub fn from_handle(handle: HandleID) -> Self { 
        Self(Mutex::new(Some(Socket::from_handle(handle))))
    }

    pub fn handle(&self) -> HandleID { 
        self.0.lock().as_ref().expect("Log connection closed").handle()
    }

    pub fn info(&self, message: &str) -> Result<(), Error> { self.log(LogLevel::Info, message.as_bytes()) }

    pub fn warn(&self, message: &str) -> Result<(), Error> { self.log(LogLevel::Warn, message.as_bytes()) }

    pub fn error(&self, message: &str) -> Result<(), Error> { self.log(LogLevel::Error, message.as_bytes()) }

    pub fn debug(&self, message: &str) -> Result<(), Error> { self.log(LogLevel::Debug, message.as_bytes()) }

    pub fn log(&self, level: LogLevel, message: &[u8]) -> Result<(), Error> {
        if message.is_empty() {
            return Ok(());
        }
        if message.len() > MAX_LOG_MESSAGE_BYTES {
            return Err(Error::invalid_argument("Log message exceeds 2048 bytes".into()));
        }

        let mut header = [0u8; 5];
        // 32 bit msg len + 8 bit log level + msg
        header[..4].copy_from_slice(&(message.len() as u32).to_le_bytes());
        header[4] = level as u8;

        let mut conn = self.0.lock();
        let socket = conn.as_ref()
            .ok_or_else(|| Error::end_of_stream("Log connection closed".into()))?;

        let result = socket.write_all(&header)
            .and_then(|()| socket.write_all(message));

        if result.is_err() {
            // partial frame severs the stream
            *conn = None;
        }

        result
    }
}

pub struct LogReader(HandleID);

impl LogReader {
    pub fn from_handle(handle: HandleID) -> Self { Self(handle) }
}

impl Read for LogReader {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> { sys_read(self.0, buf.as_mut_ptr(), buf.len(), 0).map_err(Error::from) }
}

impl Write for SystemLog {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        self.log(LogLevel::Info, buf)?;
        Ok(buf.len())
    }
}
