use alloc::string::String;
use vespertine_abi::typed::{FileSizeValue, ValueType};
use vespertine_cli::args::Command;
use vespertine_rt::println;
use vespertine_std::{Error, list_processes, typed::{RecordStream, TypedValue}};

pub const SYS_PROCS_LIST_SCHEMA: u64 = 1;

pub fn procs(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("list")
        .parse(args).
        map_err(Error::from)?;

    if matches.flag("help") {
        println!("usage: sys procs");
        return Ok(());
    }

    if matches.positional_count() > 0 {
        return Err(Error::invalid_argument("usage: sys procs".into()));
    }

    let mut out = RecordStream::typed_default_out(
        SYS_PROCS_LIST_SCHEMA,
        &[
            ("pid", ValueType::Integer),
            ("user", ValueType::Integer),
            ("state", ValueType::String),
            ("threads", ValueType::Integer),
            ("memory", ValueType::FileSize),
            ("reason", ValueType::Integer),
            ("code", ValueType::Integer),
            ("detail", ValueType::Integer),
        ],
        &["pid", "state"],
    )?;

    out.table(&["pid", "user", "state", "threads", "memory", "reason", "code", "detail"])?;

    let mut proc_iter = list_processes()?;

    while let Some(entry) = proc_iter.next() {
        out.row_values(&[
            TypedValue::Integer(entry.pid as i128),
            TypedValue::Integer(entry.user.0 as i128),
            TypedValue::String(entry.short_status().into()),
            TypedValue::Integer(entry.active_threads as i128),
            TypedValue::FileSize(FileSizeValue {
                bytes: entry.memory_usage as i128,
                block_size: 0, blocks: 0, flags: 0, reserved: 0,
            }),
            TypedValue::Integer(entry.term_reason as i128),
            TypedValue::Integer(entry.term_code as i128),
            TypedValue::Integer(entry.term_detail as i128),
        ])?;
    }

    out.finish()?;

    Ok(())
}
