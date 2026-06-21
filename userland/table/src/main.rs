#![no_main]
#![no_std]

use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use vespertine_abi::ProcessInitPackage;
use vespertine_abi::typed::RECORD_PRESENTATION_TABLE;
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_std::typed::{
    DisplayOptions,
    RecordFieldInfo,
    ShellValue,
    TypedReader,
    TypedValue,
};
use vespertine_std::{
    Error,
    HandleReader,
    HandleWriter,
    Write,
    env,
};

extern crate alloc;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let _pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run() {
        println!("table error: {:?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run() -> Result<(), Error> {
    let args = env::args();
    let requested = &args[1..];

    let reader = TypedReader::new(HandleReader::new(env::source()));
    let out = HandleWriter::new(env::sink());

    let mut schemas = BTreeMap::new();
    let mut presentations = BTreeMap::new();

    let mut active_schema = None;
    let mut rows = Vec::new();

    while let Some(value) = reader.next_value()? {
        match value {
            ShellValue::RecordSchema { schema_id, fields } => {
                active_schema = Some(schema_id);
                schemas.insert(schema_id, fields);
            }
            ShellValue::RecordPresentation { schema_id, presentation, fields } => {
                presentations.insert((schema_id, presentation), fields);
            }
            ShellValue::Value(TypedValue::Record { schema_id, fields }) => {
                active_schema = Some(schema_id);
                rows.push(fields);
            }
            ShellValue::Value(value) => {
                let text = value.display_with(Default::default());
                out.write_all(text.as_bytes())?;
                out.write_all(b"\n")?;
            }
            ShellValue::StreamEnd => {
                if let Some(schema_id) = active_schema {
                    render_table(
                        &out,
                        schemas.get(&schema_id),
                        presentations.get(&(schema_id, RECORD_PRESENTATION_TABLE)),
                        requested,
                        &rows,
                    )?;
                    rows.clear();
                }
            }
            ShellValue::Error(msg) => {
                return Err(Error::invalid_argument(msg));
            }
        }
    }

    if !rows.is_empty() {
        if let Some(schema_id) = active_schema {
            render_table(&out, schemas.get(&schema_id), presentations.get(&(schema_id, RECORD_PRESENTATION_TABLE)), requested, &rows)?;
        }
    }

    Ok(())
}

fn render_table<W: Write>(
    out: &W, schema: Option<&Vec<RecordFieldInfo>>, table_presentation: Option<&Vec<u16>>, requested: &[String], rows: &[Vec<TypedValue>],
) -> Result<(), Error> {
    let Some(schema) = schema else {
        return Ok(());
    };

    let columns = resolve_columns(schema, table_presentation, requested)?;

    if columns.is_empty() {
        return Ok(());
    }

    let mut widths = Vec::new();

    let opts = DisplayOptions::default();

    for idx in &columns {
        let idx = *idx as usize;
        let mut width = schema.get(idx).map(|f| f.name.len()).unwrap_or(0);

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

    for (pos, field_idx) in columns.iter().enumerate() {
        if pos > 0 {
            out.write_all(b" ")?;
        }

        let name = schema.get(*field_idx as usize).map(|f| f.name.as_str()).unwrap_or("");
        write_padded(out, name.as_bytes(), widths[pos])?;
    }

    out.write_all(b"\n")?;

    for row in rows {
        for (pos, field_idx) in columns.iter().enumerate() {
            if pos > 0 {
                out.write_all(b" ")?;
            }

            let rendered = row.get(*field_idx as usize).map(|v| v.display_with(opts)).unwrap_or_else(String::new);
            write_padded(out, rendered.as_bytes(), widths[pos])?;
        }
        out.write_all(b"\n")?;
    }

    Ok(())
}

fn resolve_columns(schema: &[RecordFieldInfo], table_presentation: Option<&Vec<u16>>, requested: &[String]) -> Result<Vec<u16>, Error> {
    if !requested.is_empty() {
        let mut columns = Vec::new();

        for name in requested {
            let Some(idx) = schema.iter().position(|f| f.name == *name) else {
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

fn write_padded<W: Write>(out: &W, bytes: &[u8], width: usize) -> Result<(), Error> {
    out.write_all(bytes)?;
    let mut remaining = width.saturating_sub(bytes.len());
    while remaining > 0 {
        out.write_all(b" ")?;
        remaining -= 1;
    }
    Ok(())
}
