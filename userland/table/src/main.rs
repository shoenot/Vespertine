#![no_main]
#![no_std]

use alloc::{collections::btree_map::BTreeMap, string::String, vec::Vec};
use vespertine_abi::{ProcessInitPackage, shell::RECORD_PRESENTATION_TABLE};
use vespertine_rt::{println, syscall::sys_close};
use vespertine_std::{Error, HandleReader, HandleWriter, Write, env, shell::{RecordFieldInfo, ShellValue, TypedReader}};

extern crate alloc;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run() {
        println!("[ERROR] table error: {:?}", e);
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
            },
            ShellValue::RecordPresentation { schema_id, presentation, fields } => {
                presentations.insert((schema_id, presentation), fields);
            },
            ShellValue::Record { schema_id, fields } => {
                active_schema = Some(schema_id);
                rows.push(fields);
            },
            ShellValue::RecordEnd { schema_id } => {
                render_table(
                    &out, 
                    schemas.get(&schema_id), 
                    presentations.get(&(schema_id, RECORD_PRESENTATION_TABLE)), 
                    requested, 
                    &rows
                )?;
                rows.clear();
            },
            ShellValue::String(value) => {
                out.write_all(value.as_bytes())?;
                out.write_all(b"\n")?;
            }
        }
    }

    if !rows.is_empty() {
        if let Some(schema_id) = active_schema {
            render_table(
                &out, 
                schemas.get(&schema_id), 
                presentations.get(&(schema_id, RECORD_PRESENTATION_TABLE)), 
                requested, 
                &rows
            )?;
        }
    }

    Ok(())
}

fn render_table<W: Write>(
    out: &W, schema: Option<&Vec<RecordFieldInfo>>, 
    table_presentation: Option<&Vec<u16>>, 
    requested: &[String], rows: &[Vec<String>]) -> Result<(), Error> {
    let Some(schema) = schema else {
        return Ok(());
    };

    let columns = resolve_columns(schema, table_presentation, requested)?;

    if columns.is_empty() {
        return Ok(());
    }

    let mut widths = Vec::new();

    for idx in &columns {
        let idx = *idx as usize;
        let mut width = schema.get(idx).map(|f| f.name.len()).unwrap_or(0);
        for row in rows {
            if let Some(cell) = row.get(idx) {
                if cell.len() > width {
                    width = cell.len();
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

            let value = row.get(*field_idx as usize).map(|v| v.as_str()).unwrap_or("");
            write_padded(out, value.as_bytes(), widths[pos])?;
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
