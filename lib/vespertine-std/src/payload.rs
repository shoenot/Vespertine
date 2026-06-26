extern crate alloc;

use crate::{Error, portal::PortalOfferId};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ------ PACKET READING ------ //

pub struct PayloadReader<'a> {
    pub bytes: &'a [u8],
    pub offset: usize,
}

impl<'a> PayloadReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self { Self { bytes, offset: 0 } }

    pub fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let end = self.offset.checked_add(length).ok_or_else(|| Error::invalid_argument("payload length overflow".into()))?;

        if end > self.bytes.len() {
            return Err(Error::invalid_argument("truncated packet payload".into()));
        }

        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    pub fn read_u8(&mut self) -> Result<u8, Error> { Ok(self.take(1)?[0]) }

    pub fn read_u16(&mut self) -> Result<u16, Error> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn read_u32(&mut self) -> Result<u32, Error> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn read_u64(&mut self) -> Result<u64, Error> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]))
    }

    pub fn read_offer_id(&mut self, description: &str) -> Result<PortalOfferId, Error> {
        let raw = self.read_u64()?;
        if raw == 0 {
            return Err(Error::invalid_argument(format!("{} is missing", description)));
        }
        usize::try_from(raw).map_err(|_| Error::invalid_argument(format!("{} is out of range", description)))
    }

    pub fn read_string(&mut self, maximum: usize, description: &str) -> Result<String, Error> {
        let length = self.read_u32()? as usize;

        if length > maximum {
            return Err(Error::invalid_argument(alloc::format!("{} is too long", description)));
        }

        let bytes = self.take(length)?;
        let value = str::from_utf8(bytes).map_err(|_| Error::invalid_encoding(alloc::format!("{} is not UTF-8", description)))?;

        Ok(value.to_string())
    }

    pub fn finish(self) -> Result<(), Error> {
        if self.offset != self.bytes.len() {
            return Err(Error::invalid_argument("packet contains trailing data".into()));
        }

        Ok(())
    }
}

// ------ PACKET WRITING ------ //

pub fn write_u8(output: &mut Vec<u8>, value: u8) { output.push(value); }

pub fn write_u16(output: &mut Vec<u8>, value: u16) { output.extend_from_slice(&value.to_le_bytes()); }

pub fn write_u32(output: &mut Vec<u8>, value: u32) { output.extend_from_slice(&value.to_le_bytes()); }

pub fn write_u64(output: &mut Vec<u8>, value: u64) { output.extend_from_slice(&value.to_le_bytes()); }

pub fn write_offer_id(output: &mut Vec<u8>, offer_id: PortalOfferId, description: &str) -> Result<(), Error> {
    if offer_id == 0 {
        return Err(Error::invalid_argument(format!("{} is missing", description)));
    }
    let raw = u64::try_from(offer_id).map_err(|_| Error::invalid_argument(format!("{} is out of range", description)))?;
    write_u64(output, raw);
    Ok(())
}

pub fn write_string(output: &mut Vec<u8>, value: &str, maximum: usize, description: &str) -> Result<(), Error> {
    if value.len() > maximum {
        return Err(Error::invalid_argument(alloc::format!("{} is too long", description)));
    }

    let length = u32::try_from(value.len()).map_err(|_| Error::invalid_argument(alloc::format!("{} is too long", description)))?;

    write_u32(output, length);
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

