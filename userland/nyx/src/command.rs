use alloc::{string::String, vec::Vec};
use vespertine_abi::{app::hesper::{AppIoMode, AppIoModes}, typed::ValueType};
use vespertine_cli::args::Command;
use vespertine_rt::println;
use vespertine_std::{Error, typed::{RecordStream, TypedValue}, vreg::VRegistryClient};

pub const NYX_INSTALLED_LIST_SCHEMA: u64 = 1;
pub const NYX_APPLICATION_INFO_SCHEMA: u64 = 2;

pub fn installed(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("installed")
        .parse(args).
        map_err(Error::from)?;

    if matches.positional_count() > 0 {
        return Err(Error::invalid_argument("\'nyx installed\' does not take any further args".into()));
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
    out.list_intent()?;
    out.table(&["name", "app_id", "installed", "updated"])?;

    let mut reg = VRegistryClient::connect()?;
    let programs_list = reg.list()?;

    for program in programs_list {
        out.row_values(&[
            TypedValue::String(program.display_name),
            TypedValue::String(program.app_id),
            TypedValue::DateTime(program.installed_ts),
            TypedValue::DateTime(program.updated_ts),
        ])?;
    }

    out.finish()?;

    Ok(())
}

pub fn info(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("info")
        .parse(args)
        .map_err(Error::from)?;

    if matches.positional_count() != 1 {
        return Err(Error::invalid_argument("usage: nyx info [app]".into()));
    }

    let name = matches.require_positional(0, "app").map_err(Error::from)?;

    let mut reg = VRegistryClient::connect()?;
    let app = reg.resolve(name)?;

    let mut out = RecordStream::typed_out(
        NYX_APPLICATION_INFO_SCHEMA,
        &[
            ("name", ValueType::String),
            ("app_id", ValueType::String),
            ("command", ValueType::String),
            ("bundle", ValueType::String),
            ("entrypoint", ValueType::String),
            ("binary", ValueType::String),
            ("input", ValueType::String),
            ("modes", ValueType::String),
            ("default_mode", ValueType::String),
            ("installed", ValueType::DateTime),
            ("updated", ValueType::DateTime),
        ],
    )?;
    out.details_intent()?;
    out.details(&[
        "name",
        "app_id",
        "command",
        "bundle",
        "entrypoint",
        "binary",
        "input",
        "modes",
        "default_mode",
        "installed",
        "updated",
    ])?;

    out.table(&[
        "name",
        "app_id",
        "entrypoint",
        "binary",
        "installed",
        "updated",
    ])?;

    out.row_values(&[
        TypedValue::String(app.display_name),
        TypedValue::String(app.app_id),
        TypedValue::String(app.command),
        TypedValue::String(app.bundle),
        TypedValue::String(app.entrypoint),
        TypedValue::String(app.binary),
        TypedValue::String(mode_name(app.input).into()),
        TypedValue::String(modes_text(app.modes)),
        TypedValue::String(mode_name(app.default_mode).into()),
        TypedValue::DateTime(app.installed_ts),
        TypedValue::DateTime(app.updated_ts),
    ])?;

    out.finish()?;
    Ok(())
}

fn mode_name(mode: AppIoMode) -> &'static str {
    match mode {
        AppIoMode::Any => "any",
        AppIoMode::Text => "text",
        AppIoMode::Typed => "typed",
        AppIoMode::Terminal => "terminal",
    }
}

fn modes_text(modes: AppIoModes) -> String {
    let mut parts = Vec::new();
    for mode in [AppIoMode::Text, AppIoMode::Typed, AppIoMode::Terminal] {
        if modes.contains_mode(mode) {
            parts.push(mode_name(mode));
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (idx, part) in parts.iter().enumerate() {
        if idx > 0 {
            out.push_str(",");
        }

        out.push_str(part);
    }
    out
}
