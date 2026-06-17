use core::slice;

use vespertine_abi::{protocol::{PacketFlags, PacketHeader, PacketType, VESPER_MAGIC}, shell::{RECORD_FIELD_NAME_MAX, RecordField, RecordHeader, RecordSchemaHeader, ValueType}};

use crate::{Error, Write};

pub struct TypedWriter<W> {
    sink: W,
}

impl<W: Write> TypedWriter<W> {
    pub fn new(sink: W) -> Self {
        Self { sink }
    }

    pub fn string(&self, s: &str) -> Result<(), Error> {
        self.write_packet(PacketType::String, s.as_bytes())
    }

    pub fn record_schema(&self, schema_id: u64, fields: &[(&str, ValueType)]) ->
    Result<(), Error> {
        if fields.len() > u16::MAX as usize {
            return Err(Error::invalid_argument("too many record fields".into()));
        }

        let payload_len = size_of::<RecordSchemaHeader>() + fields.len() *
        size_of::<RecordField>();
        let header = RecordSchemaHeader {
            schema_id,
            field_count: fields.len() as u16,
            reserved: 0,
            payload_len: payload_len as u32,
        };

        let packet = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: PacketType::RecordSchema as u32,
            payload_len: payload_len as u32,
            reserved: 0,
        };

        self.write_struct(&packet)?;
        self.write_struct(&header)?;

        for (name, value_type) in fields {
            if name.len() > RECORD_FIELD_NAME_MAX {
                return Err(Error::name_too_long("record field name too long".into()));
            }

            let mut field = RecordField {
                value_type: *value_type,
                name_len: name.len() as u8,
                reserved: 0,
                name: [0; RECORD_FIELD_NAME_MAX],
            };
            field.name[..name.len()].copy_from_slice(name.as_bytes());
            self.write_struct(&field)?;
        }

        Ok(())
    }

    pub fn record(&self, schema_id: u64, values: &[&str]) -> Result<(), Error> {
        if values.len() > u16::MAX as usize {
            return Err(Error::invalid_argument("too many record values".into()));
        }

        let mut payload_len = size_of::<RecordHeader>();
        for value in values {
            payload_len += size_of::<u32>() + value.len();
        }

        let packet = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: PacketType::Record as u32,
            payload_len: payload_len as u32,
            reserved: 0,
        };

        let record = RecordHeader {
            schema_id,
            field_count: values.len() as u16,
            reserved: 0,
        };

        self.write_struct(&packet)?;
        self.write_struct(&record)?;

        for value in values {
            let len = value.len() as u32;
            self.write_struct(&len)?;
            self.sink.write_all(value.as_bytes())?;
        }

        Ok(())
    }

    pub fn record_end(&self, schema_id: u64) -> Result<(), Error> {
        let payload = RecordHeader {
            schema_id,
            field_count: 0,
            reserved: 0,
        };

        let bytes = unsafe {
            slice::from_raw_parts(
                &payload as *const _ as *const u8,
                size_of::<RecordHeader>(),
            )
        };

        self.write_packet(PacketType::RecordEnd, bytes)
    }

    fn write_packet(&self, packet_type: PacketType, payload: &[u8]) -> Result<(), Error>
    {
        let header = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: packet_type as u32,
            payload_len: payload.len() as u32,
            reserved: 0,
        };

        self.write_struct(&header)?;
        self.sink.write_all(payload)?;
        Ok(())
    }

    fn write_struct<T: Copy>(&self, value: &T) -> Result<(), Error> {
        let bytes = unsafe {
            slice::from_raw_parts(value as *const _ as *const u8, size_of::<T>())
        };
        self.sink.write_all(bytes)
    }
}
