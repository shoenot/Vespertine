use core::ptr::read_unaligned;
use core::{
    slice,
    str,
};
extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use vespertine_abi::protocol::{
    PacketFlags,
    PacketHeader,
    PacketType,
    VESPER_MAGIC,
};
use vespertine_abi::typed::{
    DateTimeValue, DateValue, FileSizeValue, ListHeader, RECORD_FIELD_NAME_MAX, RecordField, RecordPresentationHeader, RecordSchemaHeader, RecordValueHeader, StreamIntentHeader, TimeValue, UserValue, ValueHeader, ValueType
};

use crate::typed::TypedValue;
use crate::{
    Error,
    ErrorKind,
    Read,
    Write,
};

#[derive(Debug, Clone)]
pub enum ShellValue {
    StreamIntent { intent: u16, flags: u16 },
    Value(TypedValue),
    RecordSchema { schema_id: u64, fields: Vec<RecordFieldInfo> },
    RecordPresentation { schema_id: u64, presentation: u16, fields: Vec<u16> },
    StreamEnd,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct RecordFieldInfo {
    pub name: String,
    pub ty: ValueType,
}

#[derive(Debug, Clone)]
pub struct RecordFieldSpec<'a> {
    pub name: &'a str,
    pub ty: ValueType,
    pub display_default: bool,
    pub display_table: bool,
}

pub struct TypedWriter<W> {
    sink: W,
}

impl<W: Write> TypedWriter<W> {
    pub fn new(sink: W) -> Self { Self { sink } }

    pub fn stream_end(&self) -> Result<(), Error> {
        let packet = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: PacketType::StreamEnd as u32,
            payload_len: 0,
            reserved: 0,
        };

        self.write_struct(&packet)
    }

    pub fn record_presentation(&self, schema_id: u64, presentation: u16, fields: &[u16]) -> Result<(), Error> {
        if fields.len() > u16::MAX as usize {
            return Err(Error::invalid_argument("too many presentation fields".into()));
        }

        let payload_len = size_of::<RecordPresentationHeader>() + fields.len() * size_of::<u16>();

        let packet = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: PacketType::RecordPresentation as u32,
            payload_len: payload_len as u32,
            reserved: 0,
        };

        let header = RecordPresentationHeader { schema_id, presentation, field_count: fields.len() as u16, reserved: 0 };

        self.write_struct(&packet)?;
        self.write_struct(&header)?;

        for field in fields {
            self.write_struct(field)?;
        }

        Ok(())
    }

    fn write_struct<T: Copy>(&self, value: &T) -> Result<(), Error> {
        let bytes = unsafe { slice::from_raw_parts(value as *const _ as *const u8, size_of::<T>()) };
        self.sink.write_all(bytes)
    }

    pub fn value(&self, value: &TypedValue) -> Result<(), Error> {
        let mut payload = Vec::new();
        encode_value(value, &mut payload)?;

        let packet = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: PacketType::Value as u32,
            payload_len: payload.len() as u32,
            reserved: 0,
        };

        self.write_struct(&packet)?;
        self.sink.write_all(&payload)
    }

    pub fn record_schema(&self, schema_id: u64, fields: &[(&str, ValueType)]) -> Result<(), Error> {
        if fields.len() > u16::MAX as usize {
            return Err(Error::invalid_argument("too many record fields".into()));
        }

        let payload_len = size_of::<RecordSchemaHeader>() + fields.len() * size_of::<RecordField>();

        let packet = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: PacketType::RecordSchema as u32,
            payload_len: payload_len as u32,
            reserved: 0,
        };

        let header = RecordSchemaHeader { schema_id, field_count: fields.len() as u16, reserved: 0 };

        self.write_struct(&packet)?;
        self.write_struct(&header)?;

        for (name, value_type) in fields {
            if name.len() > RECORD_FIELD_NAME_MAX {
                return Err(Error::name_too_long("record field name too long".into()));
            }

            let mut field =
                RecordField { value_type: value_type.as_u16(), name_len: name.len() as u8, reserved: 0, name: [0; RECORD_FIELD_NAME_MAX] };

            field.name[..name.len()].copy_from_slice(name.as_bytes());
            self.write_struct(&field)?;
        }

        Ok(())
    }

    pub fn stream_intent(&self, intent: u16, flags: u16) -> Result<(), Error> {
        let packet = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: PacketType::StreamIntent as u32,
            payload_len: size_of::<StreamIntentHeader>() as u32,
            reserved: 0,
        };

        let header = StreamIntentHeader { intent, flags, reserved: 0 };

        self.write_struct(&packet)?;
        self.write_struct(&header)
    }

    pub fn error(&self, message: &str) -> Result<(), Error> {
        let packet = PacketHeader {
            magic: VESPER_MAGIC,
            version: 1,
            packet_flags: PacketFlags::IS_BUFFER,
            packet_type: PacketType::ShellError as u32,
            payload_len: message.len() as u32,
            reserved: 0,
        };

        self.write_struct(&packet)?;
        self.sink.write_all(message.as_bytes())
    }
}

fn push_struct<T: Copy>(out: &mut Vec<u8>, value: &T) {
    let bytes = unsafe { core::slice::from_raw_parts(value as *const _ as *const u8, size_of::<T>()) };
    out.extend_from_slice(bytes);
}

fn encode_value(value: &TypedValue, out: &mut Vec<u8>) -> Result<(), Error> {
    let mut payload = Vec::new();

    let ty = match value {
        TypedValue::String(s) => {
            payload.extend_from_slice(s.as_bytes());
            ValueType::String
        }
        TypedValue::Integer(v) => {
            push_struct(&mut payload, v);
            ValueType::Integer
        }
        TypedValue::Float(v) => {
            push_struct(&mut payload, v);
            ValueType::Float
        }
        TypedValue::Bool(v) => {
            payload.push(if *v { 1 } else { 0 });
            ValueType::Bool
        }
        TypedValue::Date(v) => {
            push_struct(&mut payload, v);
            ValueType::Date
        }
        TypedValue::Time(v) => {
            push_struct(&mut payload, v);
            ValueType::Time
        }
        TypedValue::DateTime(v) => {
            push_struct(&mut payload, v);
            ValueType::DateTime
        }
        TypedValue::FileSize(v) => {
            push_struct(&mut payload, v);
            ValueType::FileSize
        }
        TypedValue::User(v) => {
            push_struct(&mut payload, v);
            ValueType::User
        }
        TypedValue::List { element_type, items } => {
            for item in items {
                encode_value(item, &mut payload)?;
            }
            let header =
                ListHeader { element_type: element_type.as_u16(), flags: 0, count: items.len() as u32, payload_len: payload.len() as u32 };
            let mut wrapped = Vec::new();
            push_struct(&mut wrapped, &header);
            wrapped.extend_from_slice(&payload);
            payload = wrapped;
            ValueType::List
        }
        TypedValue::Record { schema_id, fields } => {
            for field in fields {
                encode_value(field, &mut payload)?;
            }
            let header =
                RecordValueHeader { schema_id: *schema_id, field_count: fields.len() as u16, flags: 0, payload_len: payload.len() as u32 };
            let mut wrapped = Vec::new();
            push_struct(&mut wrapped, &header);
            wrapped.extend_from_slice(&payload);
            payload = wrapped;
            ValueType::Record
        }
    };

    let header = ValueHeader { value_type: ty.as_u16(), flags: 0, payload_len: payload.len() as u32 };

    push_struct(out, &header);
    out.extend_from_slice(&payload);
    Ok(())
}

pub struct TypedReader<R> {
    source: R,
}

impl<R: Read> TypedReader<R> {
    pub fn new(source: R) -> Self { Self { source } }

    pub fn next_value(&self) -> Result<Option<ShellValue>, Error> {
        let mut header = PacketHeader::default();

        match self.read_struct(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind == ErrorKind::EndOfStream => return Ok(None),
            Err(error) => return Err(error),
        }

        if header.magic != VESPER_MAGIC {
            return Err(Error::invalid_argument("invalid typed packet magic".into()));
        }

        match header.packet_type {
            x if x == PacketType::RecordSchema as u32 => Ok(Some(self.read_record_schema()?)),
            x if x == PacketType::RecordPresentation as u32 => Ok(Some(self.read_record_presentation()?)),
            x if x == PacketType::Value as u32 => {
                let mut payload = Vec::new();
                payload.resize(header.payload_len as usize, 0);
                self.source.read_exact(&mut payload)?;
                let (value, used) = decode_value(&payload)?;
                if used != payload.len() {
                    return Err(Error::invalid_argument("trailing typed value bytes".into()));
                }
                Ok(Some(ShellValue::Value(value)))
            },
            x if x == PacketType::StreamEnd as u32 => {
                if header.payload_len != 0 {
                    return Err(Error::invalid_argument("stream_end packet had payload".into()));
                }
                Ok(Some(ShellValue::StreamEnd))
            },
            x if x == PacketType::StreamIntent as u32 => Ok(Some(self.read_stream_intent()?)),
            x if x == PacketType::ShellError as u32 => {
                let message = self.read_string(header.payload_len)?;
                Ok(Some(ShellValue::Error(message)))
            }
            _ => Err(Error::invalid_argument(format!("unknown typed packet type: {}", header.packet_type).into())),
        }
    }

    fn read_record_schema(&self) -> Result<ShellValue, Error> {
        let mut header = RecordSchemaHeader { schema_id: 0, field_count: 0, reserved: 0 };
        self.read_struct(&mut header)?;

        let mut fields = Vec::new();

        for _ in 0..header.field_count {
            let mut field =
                RecordField { value_type: ValueType::String.as_u16(), name_len: 0, reserved: 0, name: [0; RECORD_FIELD_NAME_MAX] };
            self.read_struct(&mut field)?;

            let name_len = field.name_len as usize;
            if name_len > RECORD_FIELD_NAME_MAX {
                return Err(Error::invalid_argument("invalid record field name length".into()));
            }

            let name = str::from_utf8(&field.name[..name_len])
                .map_err(|_| Error::invalid_argument("record field name was not utf-8".into()))?
                .into();

            let value_type =
                ValueType::from_u16(field.value_type).ok_or_else(|| Error::invalid_argument("unknown record field type".into()))?;
            fields.push(RecordFieldInfo { name, ty: value_type });
        }

        Ok(ShellValue::RecordSchema { schema_id: header.schema_id, fields })
    }

    fn read_record_presentation(&self) -> Result<ShellValue, Error> {
        let mut header = RecordPresentationHeader { schema_id: 0, presentation: 0, field_count: 0, reserved: 0 };

        self.read_struct(&mut header)?;

        let mut fields = Vec::new();

        for _ in 0..header.field_count {
            let mut field = 0u16;
            self.read_struct(&mut field)?;
            fields.push(field);
        }

        Ok(ShellValue::RecordPresentation { schema_id: header.schema_id, presentation: header.presentation, fields })
    }

    fn read_string(&self, len: u32) -> Result<String, Error> {
        let mut buf = Vec::new();
        buf.resize(len as usize, 0);

        self.source.read_exact(&mut buf)?;

        String::from_utf8(buf).map_err(|_| Error::invalid_argument("typed string was not utf-8".into()))
    }

    fn read_struct<T: Copy>(&self, value: &mut T) -> Result<(), Error> {
        let bytes = unsafe { slice::from_raw_parts_mut(value as *mut _ as *mut u8, size_of::<T>()) };

        self.source.read_exact(bytes)
    }

    fn read_stream_intent(&self) -> Result<ShellValue, Error> {
        let mut header = StreamIntentHeader { intent: 0, flags: 0, reserved: 0 };
        self.read_struct(&mut header)?;
        Ok(ShellValue::StreamIntent { intent: header.intent, flags: header.flags })
    }
}

fn decode_value(buf: &[u8]) -> Result<(TypedValue, usize), Error> {
    if buf.len() < size_of::<ValueHeader>() {
        return Err(Error::invalid_argument("short value header".into()));
    }

    let header = read_prefix::<ValueHeader>(buf)?;
    let start = size_of::<ValueHeader>();
    let end = start + header.payload_len as usize;

    if end > buf.len() {
        return Err(Error::invalid_argument("short value payload".into()));
    }

    let payload = &buf[start..end];

    let value_type = ValueType::from_u16(header.value_type).ok_or_else(|| Error::invalid_argument("unknown value type".into()))?;

    let value = match value_type {
        ValueType::String => TypedValue::String(
            String::from_utf8(payload.to_vec()).map_err(|_| Error::invalid_argument("typed string was not utf-8".into()))?,
        ),
        ValueType::Integer => read_copy::<i128>(payload).map(TypedValue::Integer)?,
        ValueType::Float => read_copy::<f64>(payload).map(TypedValue::Float)?,
        ValueType::Bool => {
            if payload.len() != 1 {
                return Err(Error::invalid_argument("bool payload size mismatch".into()));
            }
            TypedValue::Bool(payload[0] != 0)
        }
        ValueType::Date => read_copy::<DateValue>(payload).map(TypedValue::Date)?,
        ValueType::Time => read_copy::<TimeValue>(payload).map(TypedValue::Time)?,
        ValueType::DateTime => read_copy::<DateTimeValue>(payload).map(TypedValue::DateTime)?,
        ValueType::FileSize => read_copy::<FileSizeValue>(payload).map(TypedValue::FileSize)?,
        ValueType::User => read_copy::<UserValue>(payload).map(TypedValue::User)?,
        ValueType::List => decode_list(payload)?,
        ValueType::Record => decode_record(payload)?,
    };

    Ok((value, end))
}

fn read_copy<T: Copy>(buf: &[u8]) -> Result<T, Error> {
    if buf.len() != size_of::<T>() {
        return Err(Error::invalid_argument("typed payload size mismatch".into()));
    }
    Ok(unsafe { read_unaligned(buf.as_ptr() as *const T) })
}

fn decode_list(payload: &[u8]) -> Result<TypedValue, Error> {
    let header = read_prefix::<ListHeader>(payload)?;
    let mut offset = size_of::<ListHeader>();
    let end = offset + header.payload_len as usize;
    if end > payload.len() {
        return Err(Error::invalid_argument("typed list payload was truncated".into()));
    }
    let mut items = Vec::new();

    for _ in 0..header.count {
        let (item, used) = decode_value(&payload[offset..end])?;
        offset += used;
        items.push(item);
    }

    let element_type =
        ValueType::from_u16(header.element_type).ok_or_else(|| Error::invalid_argument("unknown list element type".into()))?;

    Ok(TypedValue::List { element_type, items })
}

fn decode_record(payload: &[u8]) -> Result<TypedValue, Error> {
    let header = read_prefix::<RecordValueHeader>(payload)?;
    let mut offset = size_of::<RecordValueHeader>();
    let end = offset + header.payload_len as usize;
    if end > payload.len() {
        return Err(Error::invalid_argument("typed record payload was truncated".into()));
    }
    let mut fields = Vec::new();

    for _ in 0..header.field_count {
        let (field, used) = decode_value(&payload[offset..end])?;
        offset += used;
        fields.push(field);
    }

    Ok(TypedValue::Record { schema_id: header.schema_id, fields })
}

fn read_prefix<T: Copy>(buf: &[u8]) -> Result<T, Error> {
    if buf.len() < size_of::<T>() {
        return Err(Error::invalid_argument("typed payload header too short".into()));
    }
    Ok(unsafe { read_unaligned(buf.as_ptr() as *const T) })
}

