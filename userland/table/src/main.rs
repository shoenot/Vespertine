#![no_main]
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use vespertine_abi::ProcessInitPackage;
use vespertine_abi::typed::{
    RECORD_PRESENTATION_TABLE,
    STREAM_INTENT_LIST,
};
use vespertine_cli::args::{
    Command,
    Opt,
};
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_std::typed::{
    RecordFieldInfo,
    ShellValue,
    TypedReader,
    TypedValue,
    TypedWriter,
};
use vespertine_std::{
    Error,
    HandleReader,
    HandleWriter,
    env,
};

static TABLE_OPTIONS: &[Opt] = &[
    Opt::flag("help", Some('h'), Some("help")),
];

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let _pkg = unsafe { &*pkg_ptr };

    if let Err(error) = run() {
        println!("table error: {:?}", error);
    }

    let _ = sys_close(env::sink());
}

fn run() -> Result<(), Error> {
    let args = env::args();
    let matches = Command::new("table")
        .options(TABLE_OPTIONS)
        .parse(&args[1..])
        .map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: table [columns...]");
        return Ok(());
    }

    let requested = matches.positionals();

    let reader = TypedReader::new(HandleReader::new(env::source()));
    let writer = TypedWriter::new(HandleWriter::new(env::sink()));

    writer.stream_intent(STREAM_INTENT_LIST, 0)?;

    let mut active_schema = None;
    let mut replaced_table_presentation = false;

    while let Some(value) = reader.next_value()? {
        match value {
            ShellValue::StreamIntent { .. } => {},
            ShellValue::RecordSchema { schema_id, fields } => {
                active_schema = Some(schema_id);
                write_schema(&writer, schema_id, &fields)?;
                if !requested.is_empty() {
                    let table_fields = resolve_fields(&fields, requested)?;
                    writer.record_presentation(schema_id, RECORD_PRESENTATION_TABLE, &table_fields)?;
                    replaced_table_presentation = true;
                }
            },
            ShellValue::RecordPresentation { schema_id, presentation, fields } => {
                if replaced_table_presentation && Some(schema_id) == active_schema && presentation == RECORD_PRESENTATION_TABLE {
                    continue;
                }
                writer.record_presentation(schema_id, presentation, &fields)?;
            },
            ShellValue::Value(value) => {
                writer.value(&value)?;
            },
            ShellValue::StreamEnd => {
                writer.stream_end()?;
                break;
            },
            ShellValue::Error(message) => {
                writer.error(&message)?;
                writer.stream_end()?;
                break;
            },
        }
    }

    Ok(())
}

fn write_schema(writer: &TypedWriter<HandleWriter>, schema_id: u64, fields: &[RecordFieldInfo]) -> Result<(), Error> {
    let specs = fields.iter()
        .map(|field| (field.name.as_str(), field.ty))
        .collect::<Vec<_>>();

    writer.record_schema(schema_id, &specs)
}

fn resolve_fields(schema: &[RecordFieldInfo], requested: &[&str]) -> Result<Vec<u16>, Error> {
    let mut fields = Vec::new();
    for name in requested {
        let Some(idx) = schema.iter().position(|field| field.name.as_str() == *name) else {
            return Err(Error::invalid_argument("unknown table column".into()));
        };
        fields.push(idx as u16);
    }
    Ok(fields)
}
