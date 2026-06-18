#![no_std]
#![no_main]

use alloc::string::{String, ToString};
use chrono::{DateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use vespertine_abi::{
    ProcessInitPackage,
    shell::{DATETIME_HAS_OFFSET, DateTimeValue},
};
use vespertine_cli::args::{Command, Opt};
use vespertine_rt::{println, syscall::sys_close};
use vespertine_std::{
    Error, ErrorKind,
    clock::Time,
    env,
    shell::TypedWriter,
    value::{DateTimeStyle, TypedValue, datetime_display},
};

extern crate alloc;

static NOW_OPTIONS: &[Opt] = &[
    Opt::value("display", Some('d'), Some("display")),
    Opt::value("timezone", Some('z'), Some("timezone")),
];

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] dt error: {:?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let args = env::args();
    let (ts, _) = Time::now();
    let dt = DateTime::from_timestamp_secs(ts as i64)
        .ok_or(0)
        .map_err(|_| Error {
            kind: ErrorKind::Unknown,
            message: "Error converting timestamp to dt".into(),
        })?;

    let command_args = &args[2..];

    match args.get(1) {
        None => dt_now(dt, command_args)?,
        Some(cmd) => match cmd.as_str() {
            "now" => dt_now(dt, command_args)?,
            _ => return Err(Error::invalid_argument("usage: dt [command] [args]".into())),
        },
    }

    Ok(())
}

fn dt_now(dt: DateTime<Utc>, args: &[String]) -> Result<(), Error> {
    let out = TypedWriter::out();
    let matches = Command::new("now")
        .options(NOW_OPTIONS)
        .parse(args)
        .map_err(Error::from)?;

    let display = match matches.value("display") {
        None | Some("iso") => DateTimeStyle::Iso,
        Some("unix") => DateTimeStyle::Unix,
        Some("date") => DateTimeStyle::Date,
        Some("time") => DateTimeStyle::Time,
        Some(_) => return Err(Error::invalid_argument("invalid display style".into())),
    };

    let value = if let Some(tz_name) = matches.value("timezone") {
        let tz: Tz = tz_name
            .parse()
            .map_err(|_| Error::invalid_argument("invalid timezone".into()))?;
        let localized = dt.with_timezone(&tz);
        tz_to_value(&localized)
    } else {
        utc_to_value(&dt)
    };

    match display {
        DateTimeStyle::Iso => {
            out.value(&TypedValue::DateTime(value))?;
        }
        DateTimeStyle::Unix => {
            out.value(&TypedValue::Integer(value.unix_seconds))?;
        }
        DateTimeStyle::Date | DateTimeStyle::Time => {
            let text = datetime_display(value).style(display).to_string();
            out.value(&TypedValue::String(text))?;
        }
    }
    Ok(())
}

fn utc_to_value(dt: &DateTime<Utc>) -> DateTimeValue {
    DateTimeValue {
        unix_seconds: dt.timestamp() as i128,
        nanos: dt.timestamp_subsec_nanos(),
        offset_minutes: 0,
        flags: DATETIME_HAS_OFFSET,
        calendar: 0,
        reserved: 0,
    }
}

fn tz_to_value<Tz: TimeZone>(dt: &DateTime<Tz>) -> DateTimeValue {
    DateTimeValue {
        unix_seconds: dt.timestamp() as i128,
        nanos: dt.timestamp_subsec_nanos(),
        offset_minutes: dt.offset().fix().local_minus_utc() / 60,
        flags: DATETIME_HAS_OFFSET,
        calendar: 0,
        reserved: 0,
    }
}
