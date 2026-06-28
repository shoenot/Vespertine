#![no_main]
#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use vespertine_abi::ProcessInitPackage;
use vespertine_abi::typed::STREAM_INTENT_DETAILS;
use vespertine_cli::args::{
    Command,
    Opt,
};
use vespertine_rt::println;
use vespertine_rt::syscall::sys_close;
use vespertine_std::typed::{
    RecordFieldInfo as StdRecordFieldInfo,
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

static SELECT_OPTIONS: &[Opt] = &[
    Opt::flag("help", Some('h'), Some("help")),
];

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let _pkg = unsafe { &*pkg_ptr };

    if let Err(error) = run() {
        println!("select error: {:?}", error);
    }

    let _ = sys_close(env::sink());
}

struct Presentation {
    schema_id: u64,
    presentation: u16,
    fields: Vec<u16>,
}

struct Record {
    schema_id: u64,
    fields: Vec<TypedValue>,
}

fn run() -> Result<(), Error> {
    let args = env::args();
    let matches = Command::new("select")
        .options(SELECT_OPTIONS)
        .parse(&args[1..])
        .map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: select [index]");
        return Ok(());
    }

    if matches.positional_count() != 1 {
        return Err(Error::invalid_argument("usage: select [index]".into()));
    }

    let index = parse_index(matches.require_positional(0, "index").map_err(Error::from)?)?;

    let reader = TypedReader::new(HandleReader::new(env::source()));

    let mut schemas = BTreeMap::new();
    let mut presentations = Vec::new();
    let mut records = Vec::new();

    while let Some(value) = reader.next_value()? {
        match value {
            ShellValue::StreamIntent { .. } => {},
            ShellValue::RecordSchema { schema_id, fields } => {
                schemas.insert(schema_id, fields);
            },
            ShellValue::RecordPresentation { schema_id, presentation, fields } => {
                presentations.push(Presentation { schema_id, presentation, fields });
            },
            ShellValue::Value(TypedValue::Record { schema_id, fields }) => {
                records.push(Record { schema_id, fields });
            },
            ShellValue::Value(_) => {
                return Err(Error::invalid_argument("select expected a record stream".into()));
            },
            ShellValue::StreamEnd => {
                break;
            },
            ShellValue::Error(message) => {
                return Err(Error::invalid_argument(message));
            },
        }
    }

    let Some(record) = records.get(index) else {
        return Err(Error::invalid_argument("selected index is out of range".into()));
    };

    let Some(schema) = schemas.get(&record.schema_id) else {
        return Err(Error::invalid_argument("selected record has no schema".into()));
    };

    let writer = TypedWriter::new(HandleWriter::new(env::sink()));

    writer.stream_intent(STREAM_INTENT_DETAILS, 0)?;
    write_schema(&writer, record.schema_id, schema)?;

    for presentation in &presentations {
        if presentation.schema_id == record.schema_id {
            writer.record_presentation(presentation.schema_id, presentation.presentation, &presentation.fields)?;
        }
    }

    writer.value(&TypedValue::Record {
        schema_id: record.schema_id,
        fields: record.fields.clone(),
    })?;

    writer.stream_end()?;

    Ok(())
}

fn parse_index(value: &str) -> Result<usize, Error> {
    if value.is_empty() {
        return Err(Error::invalid_argument("index cannot be empty".into()));
    }

    let mut index = 0usize;

    for byte in value.as_bytes() {
        if !byte.is_ascii_digit() {
            return Err(Error::invalid_argument("index must be a non-negative integer".into()));
        }

        index = index
            .checked_mul(10)
            .and_then(|value| value.checked_add((byte - b'0') as usize))
            .ok_or_else(|| Error::invalid_argument("index is too large".into()))?;
    }

    Ok(index)
}

fn write_schema(writer: &TypedWriter<HandleWriter>, schema_id: u64, fields: &[StdRecordFieldInfo]) -> Result<(), Error> {
    let specs = fields.iter()
        .map(|field| (field.name.as_str(), field.ty))
        .collect::<Vec<_>>();

    writer.record_schema(schema_id, &specs)
}
