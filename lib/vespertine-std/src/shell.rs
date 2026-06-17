use core::{mem::zeroed, slice, str};
extern crate alloc;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::collections::BTreeMap;
use vespertine_abi::{protocol::{PacketFlags, PacketHeader, PacketType, VESPER_MAGIC}, shell::{RECORD_FIELD_NAME_MAX, RECORD_PRESENTATION_DEFAULT, RECORD_PRESENTATION_TABLE, RecordField, RecordHeader, RecordPresentationHeader, RecordSchemaHeader, ValueType}};
use crate::{Error, ErrorKind, HandleWriter, Read, Write, env};

#[derive(Debug, Clone)]
pub enum ShellValue {
    String(String),
    RecordSchema {
        schema_id: u64,
        fields: Vec<RecordFieldInfo>
    },
    RecordPresentation {
        schema_id: u64,
        presentation: u16,
        fields: Vec<u16>,
    },
    Record {
        schema_id: u64,
        fields: Vec<String>,
    },
    RecordEnd {
        schema_id: u64,
    }
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
    pub fn new(sink: W) -> Self {
        Self { sink }
    }

    pub fn string(&self, s: &str) -> Result<(), Error> {
        self.write_packet(PacketType::String, s.as_bytes())
    }

    pub fn record_schema(&self, schema_id: u64, fields: &[(&str, ValueType)]) -> Result<(), Error> {
        if fields.len() > u16::MAX as usize {
            return Err(Error::invalid_argument("too many record fields".into()));
        }

        let payload_len = size_of::<RecordSchemaHeader>() + fields.len() * size_of::<RecordField>();
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

        let header = RecordPresentationHeader {
            schema_id,
            presentation,
            field_count: fields.len() as u16,
            reserved: 0,
        };
    
        self.write_struct(&packet)?;
        self.write_struct(&header)?;
    
        for field in fields {
            self.write_struct(field)?;
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

pub struct TypedReader<R> {
    source: R,
}

impl<R: Read> TypedReader<R> {
    pub fn new(source: R) -> Self {
        Self { source }
    }

    pub fn next_value(&self) -> Result<Option<ShellValue>, Error> {
        let mut header = PacketHeader::default();

        match self.read_struct(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind == ErrorKind::OutOfMemory => return Ok(None),
            Err(error) => return Err(error),
        }

        if header.magic != VESPER_MAGIC {
            return Err(Error::invalid_argument("invalid typed packet magic".into()));
        }

        match header.packet_type {
            x if x == PacketType::String as u32 => {
                Ok(Some(ShellValue::String(self.read_string(header.payload_len)?)))
            },
            x if x == PacketType::RecordSchema as u32 => {
                Ok(Some(self.read_record_schema()?))
            },
            x if x == PacketType::RecordPresentation as u32 => {
                Ok(Some(self.read_record_presentation()?))
            },
            x if x == PacketType::Record as u32 => {
                Ok(Some(self.read_record()?))
            },
            x if x == PacketType::RecordEnd as u32 => {
                Ok(Some(self.read_record_end()?))
            },

            _ => Err(Error::invalid_argument("unknown typed packet type".into())),
        }
    }

    fn read_record_schema(&self) -> Result<ShellValue, Error> {
        let mut header = RecordSchemaHeader {
            schema_id: 0,
            field_count: 0,
            reserved: 0,
            payload_len: 0,
        };
        self.read_struct(&mut header)?;

        let mut fields = Vec::new();

        for _ in 0..header.field_count {
            let mut field = RecordField {
                value_type: ValueType::String,
                name_len: 0,
                reserved: 0,
                name: [0; RECORD_FIELD_NAME_MAX],
            };
            self.read_struct(&mut field)?;

            let name_len = field.name_len as usize;
            if name_len > RECORD_FIELD_NAME_MAX {
                return Err(Error::invalid_argument("invalid record field name length".into()));
            }

            let name = str::from_utf8(&field.name[..name_len])
                .map_err(|_| Error::invalid_argument("record field name was not utf-8".into()))?
                .into();

            fields.push(RecordFieldInfo {
                name,
                ty: field.value_type,
            });
        }

        Ok(ShellValue::RecordSchema {
            schema_id: header.schema_id,
            fields,
        })
    }

    fn read_record_presentation(&self) -> Result<ShellValue, Error> {
        let mut header = RecordPresentationHeader {
            schema_id: 0,
            presentation: 0,
            field_count: 0,
            reserved: 0,
        };
    
        self.read_struct(&mut header)?;
    
        let mut fields = Vec::new();
    
        for _ in 0..header.field_count {
            let mut field = 0u16;
            self.read_struct(&mut field)?;
            fields.push(field);
        }
    
        Ok(ShellValue::RecordPresentation {
            schema_id: header.schema_id,
            presentation: header.presentation,
            fields,
        })
    }

    fn read_record(&self) -> Result<ShellValue, Error> {
        let mut header = RecordHeader {
            schema_id: 0,
            field_count: 0,
            reserved: 0,
        };

        self.read_struct(&mut header)?;

        let mut fields = Vec::new();

        for _ in 0..header.field_count {
            let mut len = 0u32;
            self.read_struct(&mut len)?;
            fields.push(self.read_string(len)?);
        }

        Ok(ShellValue::Record {
            schema_id: header.schema_id,
            fields,
        })
    }

    fn read_record_end(&self) -> Result<ShellValue, Error> {
        let mut header = RecordHeader {
            schema_id: 0,
            field_count: 0,
            reserved: 0,
        };

        self.read_struct(&mut header)?;

        Ok(ShellValue::RecordEnd {
            schema_id: header.schema_id,
        })
    }

    fn read_string(&self, len: u32) -> Result<String, Error> {
        let mut buf = Vec::new();
        buf.resize(len as usize, 0);

        self.source.read_exact(&mut buf)?;

        String::from_utf8(buf)
            .map_err(|_| Error::invalid_argument("typed string was not utf-8".into()))
    }

    fn read_struct<T: Copy>(&self, value: &mut T) -> Result<(), Error> {
        let bytes = unsafe {
            slice::from_raw_parts_mut(value as *mut _ as *mut u8, size_of::<T>())
        };

        self.source.read_exact(bytes)
    }
}

pub struct TerminalRenderer<W> {
    out: W,
    schemas: BTreeMap<u64, Vec<RecordFieldInfo>>,
    presentations: BTreeMap<(u64, u16), Vec<u16>>,
}

impl<W: Write> TerminalRenderer<W> {
    pub fn new(out: W) -> Self {
        Self { 
            out, 
            schemas: BTreeMap::new(),
            presentations: BTreeMap::new(),
        }
    }

    pub fn render(&mut self, value: ShellValue) -> Result<(), Error> {
        match value {
            ShellValue::String(value) => {
                self.out.write_all(value.as_bytes())?;
                self.out.write_all(b"\n")
            },
            ShellValue::RecordSchema { schema_id, fields } => {
                self.schemas.insert(schema_id, fields);
                Ok(())
            },
            ShellValue::RecordPresentation { schema_id, presentation, fields } => {
                self.presentations.insert((schema_id, presentation), fields);
                Ok(())
            }
            ShellValue::Record { schema_id, fields } => {
                self.render_record(schema_id, &fields)
            },
            ShellValue::RecordEnd { .. } => Ok(()),
        }
    }

    fn render_record(&mut self, schema_id: u64, values: &[String]) -> Result<(), Error> {
        if let Some(fields) = self.presentations.get(&(schema_id, RECORD_PRESENTATION_DEFAULT)) {
            let mut printed = false;
    
            for field in fields {
                let index = *field as usize;
    
                if let Some(value) = values.get(index) {
                    if printed {
                        self.out.write_all(b" ")?;
                    }
    
                    self.out.write_all(value.as_bytes())?;
                    printed = true;
                }
            }
    
            if printed {
                return self.out.write_all(b"\n");
            }
        }
    
        if let Some(first) = values.first() {
            self.out.write_all(first.as_bytes())?;
        }
    
        self.out.write_all(b"\n")
    }
}

pub fn render_typed_stream<R: Read, W: Write>(source: R, sink: W) -> Result<(), Error> {
    let reader = TypedReader::new(source);
    let mut renderer = TerminalRenderer::new(sink);

    while let Some(value) = reader.next_value()? {
        renderer.render(value)?;
    }

    Ok(())
}

impl TypedWriter<HandleWriter> {
    pub fn out() -> Self {
        Self::new(HandleWriter::new(env::sink()))
    }
}

pub struct RecordStream<W> {
    writer: TypedWriter<W>,
    schema_id: u64,
    fields: Vec<String>,
    finished: bool,
}

impl RecordStream<HandleWriter> {
    pub fn out(schema_id: u64, fields: &[&str]) -> Result<Self, Error> {
        Self::new(TypedWriter::out(), schema_id, fields)
    }

    pub fn default_out(schema_id: u64, fields: &[&str], default_fields: &[&str]) -> Result<Self, Error> {
        let stream = Self::out(schema_id, fields)?;
        stream.default(default_fields)?;
        Ok(stream)
    }
}

impl<W: Write> RecordStream<W> {
    pub fn new(writer: TypedWriter<W>, schema_id: u64, fields: &[&str]) -> Result<Self, Error> {
        let mut specs = Vec::new();

        for field in fields {
            specs.push((*field, ValueType::String));
        }

        writer.record_schema(schema_id, &specs)?;

        Ok(Self { 
            writer, 
            schema_id, 
            fields: fields.iter().map(|s| String::from(*s)).collect(), 
            finished: false 
        })
    }

    pub fn default(&self, fields: &[&str]) -> Result<(), Error> {
        self.presentation(RECORD_PRESENTATION_DEFAULT, fields)
    }

    pub fn table(&self, fields: &[&str]) -> Result<(), Error> {
        self.presentation(RECORD_PRESENTATION_TABLE, fields)
    }

    pub fn row(&self, values: &[&str]) -> Result<(), Error> {
        if values.len() != self.fields.len() {
            return Err(Error::invalid_argument("record row has wrong field count".into()));
        }
        self.writer.record(self.schema_id, values)
    }

    pub fn finish(&mut self) -> Result<(), Error> {
        if self.finished { return Ok(()); }
        self.finished = true;
        self.writer.record_end(self.schema_id)
    }

    fn presentation(&self, presentation: u16, fields: &[&str]) -> Result<(), Error> {
        let mut indices = Vec::new();

        for field in fields {
            let Some(idx) = self.fields.iter().position(|known| known == field) else {
                return Err(Error::invalid_argument("unknown record presentation format".into()));
            };
            indices.push(idx as u16);
        }
        self.writer.record_presentation(self.schema_id, presentation, &indices)
    }
}
