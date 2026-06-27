use alloc::string::String;
use vespertine_abi::typed::ValueType;
use vespertine_cli::args::Command;
use vespertine_rt::println;
use vespertine_std::{Error, typed::{RecordStream, TypedValue}, vreg::VRegistryClient};

pub const NYX_INSTALLED_LIST_SCHEMA: u64 = 1;

pub fn installed(args: &[String]) -> Result<(), Error> {
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
        NYX_INSTALLED_LIST_SCHEMA,
        &[
            ("name", ValueType::String),
            ("app_id", ValueType::String),
            ("installed", ValueType::DateTime),
            ("updated", ValueType::DateTime),
        ],
        &["name"],
    )?;

    out.table(&["name", "app_id", "installed", "updated"])?;

    let mut reg = VRegistryClient::connect()?;
    let programs_list = reg.list()?;

    for program in programs_list {
        out.row_values(&[
            TypedValue::String(program.display_name),
            TypedValue::String(program.app_id),
            TypedValue::String(program.installed_ts),
            TypedValue::String(program.updated_ts),
        ])?;
    }

    out.finish()?;

    Ok(())
}
