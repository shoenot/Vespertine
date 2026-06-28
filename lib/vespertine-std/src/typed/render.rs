extern crate alloc;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::string::ToString;
use alloc::collections::BTreeMap;
use vespertine_abi::typed::STREAM_INTENT_TABLE;
use vespertine_abi::typed::{RECORD_PRESENTATION_DEFAULT, RECORD_PRESENTATION_DETAILS, RECORD_PRESENTATION_TABLE, STREAM_INTENT_CHOICES, STREAM_INTENT_DEFAULT, STREAM_INTENT_DETAILS, STREAM_INTENT_LIST, ValueType};

use crate::{Error, HandleWriter, Read, Write, env};
use crate::typed::{DisplayOptions, RecordFieldInfo, ShellValue, TypedReader, TypedValue, TypedWriter};

pub struct TerminalRenderer<W> {
    out: W,
    intent: u16,
    schemas: BTreeMap<u64, Vec<RecordFieldInfo>>,
    presentations: BTreeMap<(u64, u16), Vec<u16>>,
    needs_separator: bool,
}

impl<W: Write> TerminalRenderer<W> {
    pub fn new(out: W) -> Self { 
        Self { 
            out, 
            intent: STREAM_INTENT_DEFAULT,
            schemas: BTreeMap::new(), 
            presentations: BTreeMap::new(), 
            needs_separator: false 
        } 
    }

    fn begin_visible_value(&mut self) -> Result<(), Error> {
        if self.needs_separator {
            self.out.write_all(b"\n")?;
        }
        self.needs_separator = true;
        Ok(())
    }

    pub fn render(&mut self, value: ShellValue) -> Result<(), Error> {
        match value {
            ShellValue::Value(v) => self.render_value(v),
            ShellValue::RecordSchema { schema_id, fields } => {
                self.schemas.insert(schema_id, fields);
                Ok(())
            },
            ShellValue::RecordPresentation { schema_id, presentation, fields } => {
                self.presentations.insert((schema_id, presentation), fields);
                Ok(())
            },
            ShellValue::StreamIntent { intent, .. } => {
                self.intent = intent;
                Ok(())
            },
            ShellValue::Error(message) => {
                self.begin_visible_value()?;
                self.out.write_all(b"error: ")?;
                self.out.write_all(message.as_bytes())
            },
            ShellValue::StreamEnd => Ok(()),
        }
    }

    fn render_value(&mut self, value: TypedValue) -> Result<(), Error> {
        self.begin_visible_value()?;

        match value {
            TypedValue::Record { schema_id, fields } => self.render_record(schema_id, &fields),
            other => {
                let text = other.display_with(Default::default());
                self.out.write_all(text.as_bytes())
            }
        }
    }


    fn render_record(&mut self, schema_id: u64, values: &[TypedValue]) -> Result<(), Error> {
        if self.intent == STREAM_INTENT_DETAILS {
            if let Some(schema) = self.schemas.get(&schema_id) {
                let rows = [values.to_vec()];
                return render_record_details(
                    &self.out,
                    schema,
                    self.presentations.get(&(schema_id, RECORD_PRESENTATION_DETAILS)),
                    &[],
                    &rows,
                    Default::default(),
                );
            }
        }
    
        if let Some(fields) = self.presentations.get(&(schema_id, RECORD_PRESENTATION_DEFAULT)) {
            let mut printed = false;
            for field in fields {
                let index = *field as usize;
    
                if let Some(value) = values.get(index) {
                    if printed {
                        self.out.write_all(b" ")?;
                    }
    
                    let rendered = value.display_with(Default::default());
                    self.out.write_all(rendered.as_bytes())?;
                    printed = true;
                }
            }
            if printed {
                return Ok(());
            }
        }
    
        if let Some(first) = values.first() {
            let rendered = first.display_with(Default::default());
            self.out.write_all(rendered.as_bytes())?;
        }
        Ok(())
    }
}

pub fn render_typed_stream<R: Read, W: Write>(source: R, sink: W) -> Result<(), Error> {
    let reader = TypedReader::new(source);
    let opts = DisplayOptions::default();
    let mut stream = BufferedRecordStream::new();

    stream.read_from(&reader, &sink, opts)?;

    let Some(schema) = stream.schema() else { return Ok(()); };

    match stream.intent {
        STREAM_INTENT_DETAILS => render_record_details(
            &sink,
            schema,
            stream.presentation(RECORD_PRESENTATION_DETAILS),
            &[],
            &stream.rows,
            opts,
        ),
        STREAM_INTENT_TABLE | STREAM_INTENT_CHOICES => render_record_table(
            &sink,
            schema,
            stream.presentation(RECORD_PRESENTATION_TABLE),
            &[],
            &stream.rows,
            opts,
        ),
        STREAM_INTENT_LIST | STREAM_INTENT_DEFAULT | _ => render_default_records(
            &sink,
            schema,
            stream.presentation(RECORD_PRESENTATION_DEFAULT),
            &stream.rows,
            opts,
        ),
    }
}

impl TypedWriter<HandleWriter> {
    pub fn out() -> Self { Self::new(HandleWriter::new(env::sink())) }
}

pub struct RecordStream<W> {
    writer: TypedWriter<W>,
    schema_id: u64,
    fields: Vec<String>,
    finished: bool,
}

impl RecordStream<HandleWriter> {
    pub fn out(schema_id: u64, fields: &[&str]) -> Result<Self, Error> { Self::new(TypedWriter::out(), schema_id, fields) }

    pub fn default_out(schema_id: u64, fields: &[&str], default_fields: &[&str]) -> Result<Self, Error> {
        let stream = Self::out(schema_id, fields)?;
        stream.default(default_fields)?;
        Ok(stream)
    }

    pub fn typed_out(schema_id: u64, fields: &[(&str, ValueType)]) -> Result<Self, Error> {
        Self::new_typed(TypedWriter::out(), schema_id, fields)
    }

    pub fn typed_default_out(schema_id: u64, fields: &[(&str, ValueType)], default_fields: &[&str]) -> Result<Self, Error> {
        let stream = Self::typed_out(schema_id, fields)?;
        stream.default(default_fields)?;
        Ok(stream)
    }
}

impl<W: Write> RecordStream<W> {
    pub fn new(writer: TypedWriter<W>, schema_id: u64, fields: &[&str]) -> Result<Self, Error> {
        let specs = fields.iter().map(|field| (*field, ValueType::String)).collect::<Vec<_>>();

        Self::new_typed(writer, schema_id, &specs)
    }

    pub fn new_typed(writer: TypedWriter<W>, schema_id: u64, fields: &[(&str, ValueType)]) -> Result<Self, Error> {
        writer.record_schema(schema_id, fields)?;

        Ok(Self { writer, schema_id, fields: fields.iter().map(|(name, _)| String::from(*name)).collect(), finished: false })
    }

    pub fn default(&self, fields: &[&str]) -> Result<(), Error> { self.presentation(RECORD_PRESENTATION_DEFAULT, fields) }

    pub fn table(&self, fields: &[&str]) -> Result<(), Error> { self.presentation(RECORD_PRESENTATION_TABLE, fields) }

    pub fn details(&self, fields: &[&str]) -> Result<(), Error> { self.presentation(RECORD_PRESENTATION_DETAILS, fields) }

    pub fn intent(&self, intent: u16) -> Result<(), Error> { self.writer.stream_intent(intent, 0) }
    
    pub fn list_intent(&self) -> Result<(), Error> { self.intent(STREAM_INTENT_LIST) }
    
    pub fn details_intent(&self) -> Result<(), Error> { self.intent(STREAM_INTENT_DETAILS) }

    pub fn table_intent(&self) -> Result<(), Error> { self.intent(STREAM_INTENT_TABLE) }
    
    pub fn choices_intent(&self) -> Result<(), Error> { self.intent(STREAM_INTENT_CHOICES) }

    pub fn row_values(&self, values: &[TypedValue]) -> Result<(), Error> {
        if values.len() != self.fields.len() {
            return Err(Error::invalid_argument("record row has wrong field count".into()));
        }

        self.writer.value(&TypedValue::Record { schema_id: self.schema_id, fields: values.to_vec() })
    }

    pub fn row(&self, values: &[&str]) -> Result<(), Error> {
        let values = values.iter().map(|value| TypedValue::String(String::from(*value))).collect::<Vec<_>>();
        self.row_values(&values)
    }

    pub fn finish(&mut self) -> Result<(), Error> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.writer.stream_end()
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

pub fn render_default_records<W: Write>(
    out: &W,
    schema: &[RecordFieldInfo],
    default_presentation: Option<&Vec<u16>>,
    rows: &[Vec<TypedValue>],
    opts: DisplayOptions,
) -> Result<(), Error> {
    let fields = if let Some(fields) = default_presentation {
        if fields.is_empty() {
            Vec::new()
        } else {
            fields.clone()
        }
    } else {
        Vec::new()
    };

    for row in rows {
        if !fields.is_empty() {
            let mut printed = false;

            for field in &fields {
                let idx = *field as usize;

                if let Some(value) = row.get(idx) {
                    if printed {
                        out.write_all(b" ")?;
                    }

                    let rendered = value.display_with(opts);
                    out.write_all(rendered.as_bytes())?;
                    printed = true;
                }
            }

            if printed {
                out.write_all(b"\n")?;
                continue;
            }
        }

        if let Some(first) = row.first() {
            let rendered = first.display_with(opts);
            out.write_all(rendered.as_bytes())?;
            out.write_all(b"\n")?;
        } else if !schema.is_empty() {
            out.write_all(b"\n")?;
        }
    }

    Ok(())
}

pub fn render_record_table<W: Write>(
    out: &W,
    schema: &[RecordFieldInfo],
    table_presentation: Option<&Vec<u16>>,
    requested: &[&str],
    rows: &[Vec<TypedValue>],
    opts: DisplayOptions,
) -> Result<(), Error> {
    let columns = resolve_table_columns(schema, table_presentation, requested)?;

    if columns.is_empty() {
        return Ok(());
    }

    let index_width = table_index_width(rows.len());

    let mut widths = Vec::new();

    for idx in &columns {
        let idx = *idx as usize;
        let mut width = schema.get(idx).map(|field| field.name.len()).unwrap_or(0);
        for row in rows {
            if let Some(cell) = row.get(idx) {
                let rendered = cell.display_with(opts);
                if rendered.len() > width {
                    width = rendered.len();
                }
            }
        }
        widths.push(width);
    }

    write_table_cell(out, b"#", index_width)?;
    
    for (pos, field_idx) in columns.iter().enumerate() {
        write_table_separator(out)?;
    
        let name = schema.get(*field_idx as usize).map(|field| field.name.as_str()).unwrap_or("");
        write_table_cell(out, name.as_bytes(), widths[pos])?;
    }
    
    out.write_all(b"\n")?;
    
    write_table_rule_cell(out, index_width)?;
    
    for width in &widths {
        write_table_rule_separator(out)?;
        write_table_rule_cell(out, *width)?;
    }
    
    out.write_all(b"\n")?;
    
    for (row_index, row) in rows.iter().enumerate() {
        let index = row_index.to_string();
    
        write_table_cell(out, index.as_bytes(), index_width)?;
    
        for (pos, field_idx) in columns.iter().enumerate() {
            write_table_separator(out)?;
    
            let rendered = row.get(*field_idx as usize).map(|value| value.display_with(opts)).unwrap_or_else(String::new);
            write_table_cell(out, rendered.as_bytes(), widths[pos])?;
        }
    
        out.write_all(b"\n")?;
    }
    Ok(())
}

pub fn render_record_details<W: Write>(
    out: &W,
    schema: &[RecordFieldInfo],
    details_presentation: Option<&Vec<u16>>,
    requested: &[&str],
    rows: &[Vec<TypedValue>],
    opts: DisplayOptions,
) -> Result<(), Error> {
    let fields = resolve_details_fields(schema, details_presentation, requested)?;

    if rows.len() <= 1 {
        if let Some(row) = rows.first() {
            render_one_record_details(out, schema, &fields, row, opts, 0)?;
        }
        return Ok(());
    }

    let title_index = title_field_index(schema);
    let body_fields = filter_title_field(&fields, title_index);

    for (row_index, row) in rows.iter().enumerate() {
        if row_index > 0 {
            out.write_all(b"\n")?;
        }
        let title = row_title(schema, row, row_index, opts);
        out.write_all(title.as_bytes())?;
        out.write_all(b"\n")?;

        if body_fields.is_empty() {
            render_one_record_details(out, schema, &fields, row, opts, 2)?;
        } else {
            render_one_record_details(out, schema, &body_fields, row, opts, 2)?;
        }
    }
    Ok(())
}

fn resolve_table_columns(schema: &[RecordFieldInfo], table_presentation: Option<&Vec<u16>>, requested: &[&str]) -> Result<Vec<u16>, Error> {
    if !requested.is_empty() {
        let mut columns = Vec::new();

        for name in requested {
            let Some(idx) = schema.iter().position(|field| field.name.as_str() == *name) else {
                return Err(Error::invalid_argument("unknown table column".into()));
            };
            columns.push(idx as u16);
        }
        return Ok(columns);
    }

    if let Some(fields) = table_presentation {
        if !fields.is_empty() {
            return Ok(fields.clone());
        }
    }
    Ok((0..schema.len()).map(|idx| idx as u16).collect())
}

fn resolve_details_fields(schema: &[RecordFieldInfo], details_presentation: Option<&Vec<u16>>, requested: &[&str]) -> Result<Vec<u16>, Error> {
    if !requested.is_empty() {
        let mut fields = Vec::new();

        for name in requested {
            let Some(idx) = schema.iter().position(|field| field.name.as_str() == *name) else {
                return Err(Error::invalid_argument("unknown details field".into()));
            };
            fields.push(idx as u16);
        }
        return Ok(fields);
    }

    if let Some(fields) = details_presentation {
        if !fields.is_empty() {
            return Ok(fields.clone());
        }
    }
    Ok((0..schema.len()).map(|idx| idx as u16).collect())
}

fn render_one_record_details<W: Write>(
    out: &W,
    schema: &[RecordFieldInfo],
    fields: &[u16],
    row: &[TypedValue],
    opts: DisplayOptions,
    indent: usize,
) -> Result<(), Error> {
    let mut name_width = 0usize;

    for field_idx in fields {
        let idx = *field_idx as usize;
        if let Some(field) = schema.get(idx) {
            if field.name.len() > name_width {
                name_width = field.name.len();
            }
        }
    }

    for field_idx in fields {
        let idx = *field_idx as usize;

        let Some(field) = schema.get(idx) else {
            continue;
        };

        let rendered = row.get(idx).map(|value|
        value.display_with(opts)).unwrap_or_else(String::new);

        write_indent(out, indent)?;
        out.write_all(field.name.as_bytes())?;

        let mut padding = name_width.saturating_sub(field.name.len());
        while padding > 0 {
            out.write_all(b" ")?;
            padding -= 1;
        }
        out.write_all(b": ")?;
        out.write_all(rendered.as_bytes())?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn title_field_index(schema: &[RecordFieldInfo]) -> Option<usize> {
    for candidate in ["name", "title", "id", "app_id", "pid", "path"] {
        if let Some(index) = schema.iter().position(|field| field.name.as_str() == candidate) {
            return Some(index);
        }
    }
    None
}

fn row_title(schema: &[RecordFieldInfo], row: &[TypedValue], row_index: usize, opts: DisplayOptions)
-> String {
    if let Some(index) = title_field_index(schema) {
        if let Some(value) = row.get(index) {
            return value.display_with(opts);
        }
    }
    let mut title = String::from("#");
    title.push_str(&row_index.to_string());
    title
}

fn filter_title_field(fields: &[u16], title_index: Option<usize>) -> Vec<u16> {
    let Some(title_index) = title_index else {
        return fields.to_vec();
    };
    fields.iter().copied().filter(|field| *field as usize != title_index).collect()
}

fn write_padded<W: Write>(out: &W, bytes: &[u8], width: usize) -> Result<(), Error> {
    out.write_all(bytes)?;
    let mut remaining = width.saturating_sub(bytes.len());
    while remaining > 0 {
        out.write_all(b" ")?;
        remaining -= 1;
    }
    Ok(())
}

fn write_table_separator<W: Write>(out: &W) -> Result<(), Error> {
    out.write_all(" │ ".as_bytes())
}

fn write_table_rule_separator<W: Write>(out: &W) -> Result<(), Error> {
    out.write_all("─┼─".as_bytes())
}

fn write_table_cell<W: Write>(out: &W, bytes: &[u8], width: usize) -> Result<(), Error> {
    out.write_all(b" ")?;
    write_padded(out, bytes, width)?;
    out.write_all(b" ")
}

fn write_table_rule_cell<W: Write>(out: &W, width: usize) -> Result<(), Error> {
    write_rule(out, width + 2)
}

fn write_rule<W: Write>(out: &W, width: usize) -> Result<(), Error> {
    for _ in 0..width {
        out.write_all("─".as_bytes())?;
    }

    Ok(())
}

fn write_indent<W: Write>(out: &W, indent: usize) -> Result<(), Error> {
    for _ in 0..indent {
        out.write_all(b" ")?;
    }
    Ok(())
}

fn table_index_width(row_count: usize) -> usize {
    let max_index = row_count.saturating_sub(1);
    let mut width = 1usize;
    let mut value = max_index;

    while value >= 10 {
        value /= 10;
        width += 1;
    }

    width
}

pub enum BufferedPush {
    Continue,
    Scalar(TypedValue),
    End,
}

pub struct BufferedRecordStream {
    pub intent: u16,
    pub schemas: BTreeMap<u64, Vec<RecordFieldInfo>>,
    pub presentations: BTreeMap<(u64, u16), Vec<u16>>,
    pub active_schema: Option<u64>,
    pub rows: Vec<Vec<TypedValue>>,
}

impl BufferedRecordStream {
    pub fn new() -> Self {
        Self {
            intent: STREAM_INTENT_DEFAULT,
            schemas: BTreeMap::new(),
            presentations: BTreeMap::new(),
            active_schema: None,
            rows: Vec::new(),
        }
    }

    pub fn push(&mut self, value: ShellValue) -> Result<BufferedPush, Error> {
        match value {
            ShellValue::StreamIntent { intent, .. } => {
                self.intent = intent;
                Ok(BufferedPush::Continue)
            },
            ShellValue::RecordSchema { schema_id, fields } => {
                self.active_schema = Some(schema_id);
                self.schemas.insert(schema_id, fields);
                Ok(BufferedPush::Continue)
            },
            ShellValue::RecordPresentation { schema_id, presentation, fields } => {
                self.presentations.insert((schema_id, presentation), fields);
                Ok(BufferedPush::Continue)
            },
            ShellValue::Value(TypedValue::Record { schema_id, fields }) => {
                self.active_schema = Some(schema_id);
                self.rows.push(fields);
                Ok(BufferedPush::Continue)
            },
            ShellValue::Value(value) => Ok(BufferedPush::Scalar(value)),
            ShellValue::StreamEnd => Ok(BufferedPush::End),
            ShellValue::Error(message) => Err(Error::invalid_argument(message)),
        }
    }

    pub fn read_from<R: Read, W: Write>(&mut self, reader: &TypedReader<R>, scalar_out: &W, opts: DisplayOptions) -> Result<(), Error> {
        while let Some(value) = reader.next_value()? {
            match self.push(value)? {
                BufferedPush::Continue => {},
                BufferedPush::Scalar(value) => {
                    let text = value.display_with(opts);
                    scalar_out.write_all(text.as_bytes())?;
                    scalar_out.write_all(b"\n")?;
                },
                BufferedPush::End => break,
            }
        }
        Ok(())
    }

    pub fn schema(&self) -> Option<&Vec<RecordFieldInfo>> {
        self.active_schema.and_then(|schema_id| self.schemas.get(&schema_id))
    }

    pub fn presentation(&self, presentation: u16) -> Option<&Vec<u16>> {
        self.active_schema.and_then(|schema_id| self.presentations.get(&(schema_id, presentation)))
    }

    pub fn clear_rows(&mut self) {
        self.rows.clear();
    }
}
