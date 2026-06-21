#![no_std]
#![no_main]

use alloc::format;
use alloc::string::{
    String,
    ToString,
};

use chrono::{
    DateTime,
    NaiveDateTime,
    Offset,
    TimeZone,
    Utc,
};
use chrono_tz::Tz;
use vespertine_abi::ProcessInitPackage;
use vespertine_abi::typed::{
    DATETIME_HAS_OFFSET,
    DateTimeValue,
};
use vespertine_cli::args::{
    Command,
    Opt,
};
use vespertine_rt::syscall::sys_close;
use vespertine_std::clock::Time;
use vespertine_std::typed::{
    DateTimeStyle,
    ShellValue,
    TypedReader,
    TypedValue,
    TypedWriter,
    datetime_display,
};
use vespertine_std::{
    Error,
    ErrorKind,
    HandleReader,
    env,
};

extern crate alloc;

static NOW_OPTIONS: &[Opt] = &[Opt::value("display", Some('d'), Some("display")), Opt::value("timezone", Some('z'), Some("timezone"))];

static FROM_OPTIONS: &[Opt] = &[Opt::value("display", Some('d'), Some("display")), Opt::value("timezone", Some('z'), Some("timezone"))];

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        let out = TypedWriter::out();
        let _ = out.error(&*format!("dt error: {:?}", e));
        let _ = out.stream_end();
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let args = env::args();
    let (ts, _) = Time::now();
    let dt = DateTime::from_timestamp_secs(ts as i64)
        .ok_or(0)
        .map_err(|_| Error { kind: ErrorKind::Unknown, message: "Error converting timestamp to dt".into() })?;

    let command_args = &args[2..];

    match args.get(1) {
        None => dt_now(dt, command_args)?,
        Some(cmd) => match cmd.as_str() {
            "now" => dt_now(dt, command_args)?,
            "from" => dt_from(command_args)?,
            _ => return Err(Error::invalid_argument("usage: dt [command] [args]".into())),
        },
    }

    Ok(())
}

fn dt_now(dt: DateTime<Utc>, args: &[String]) -> Result<(), Error> {
    let out = TypedWriter::out();
    let matches = Command::new("now").options(NOW_OPTIONS).parse(args).map_err(Error::from)?;

    let display = match matches.value("display") {
        None | Some("iso") => DateTimeStyle::Iso,
        Some("unix") => DateTimeStyle::Unix,
        Some("date") => DateTimeStyle::Date,
        Some("time") => DateTimeStyle::Time,
        Some(_) => return Err(Error::invalid_argument("invalid display style".into())),
    };

    let value = if let Some(tz_name) = matches.value("timezone") {
        let tz: Tz = tz_name.parse().map_err(|_| Error::invalid_argument("invalid timezone".into()))?;
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
            out.value(&TypedValue::Integer(value.unix_seconds as i128))?;
        }
        DateTimeStyle::Date | DateTimeStyle::Time => {
            let text = datetime_display(value).style(display).to_string();
            out.value(&TypedValue::String(text))?;
        }
    }
    Ok(())
}

fn dt_from(args: &[String]) -> Result<(), Error> {
    let out = TypedWriter::out();
    let matches = Command::new("from").options(FROM_OPTIONS).parse(args).map_err(Error::from)?;

    let display = match matches.value("display") {
        None | Some("iso") => DateTimeStyle::Iso,
        Some("unix") => DateTimeStyle::Unix,
        Some("date") => DateTimeStyle::Date,
        Some("time") => DateTimeStyle::Time,
        Some(_) => return Err(Error::invalid_argument("invalid display style".into())),
    };

    let dt = if let Some(raw) = args.get(0) {
        datetime_from_string(raw)?
    } else {
        let reader = TypedReader::new(HandleReader::new(env::source()));
        let mut dt_opt = None;
        while let Some(value) = reader.next_value()? {
            match value {
                ShellValue::Value(v) => {
                    dt_opt = Some(datetime_from_value(v)?);
                }
                ShellValue::StreamEnd => break,
                _ => {
                    return Err(Error::invalid_argument("`dt from` needs a datetime value to parse".into()));
                }
            }
        }
        match dt_opt {
            None => {
                return Err(Error::invalid_argument("could not parse piped input into DateTime".into()));
            }
            Some(v) => v,
        }
    };

    let value = if let Some(tz_name) = matches.value("timezone") {
        let tz: Tz = tz_name.parse().map_err(|_| Error::invalid_argument("invalid timezone".into()))?;
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
            out.value(&TypedValue::Integer(value.unix_seconds as i128))?;
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
        unix_seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos(),
        offset_minutes: 0,
        flags: DATETIME_HAS_OFFSET,
        calendar: 0,
        reserved: 0,
    }
}

fn tz_to_value<Tz: TimeZone>(dt: &DateTime<Tz>) -> DateTimeValue {
    DateTimeValue {
        unix_seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos(),
        offset_minutes: dt.offset().fix().local_minus_utc() / 60,
        flags: DATETIME_HAS_OFFSET,
        calendar: 0,
        reserved: 0,
    }
}

fn datetime_from_raw(raw: &String) -> Result<DateTime<Utc>, Error> {
    let trimmed = raw.trim();

    // try common formats first
    let datetime_layouts = &["%Y-%m-%d %H:%M:%S", "%d %b %Y %H:%M:%S", "%d/%m/%Y %H:%M:%S", "%m/%d/%Y %H:%M:%S", "%Y/%m/%d %H:%M:%S"];
    for &layout in datetime_layouts {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, layout) {
            return Ok(ndt.and_utc());
        }
    }
    Err(Error::invalid_argument("could not parse raw input into DateTime".into()))
}

fn datetime_from_epoch(raw: &String) -> Result<DateTime<Utc>, Error> {
    let trimmed = raw.trim();
    let number = trimmed.parse::<i64>().map_err(|_| Error::invalid_argument("could not parse raw input into DateTime".into()))?;
    if let Some(dt) = DateTime::from_timestamp(number, 0) {
        return Ok(dt);
    }
    Err(Error::invalid_argument("could not parse raw input into DateTime".into()))
}

fn datetime_from_value(value: TypedValue) -> Result<DateTime<Utc>, Error> {
    match value {
        TypedValue::DateTime(v) => DateTime::from_timestamp(v.unix_seconds as i64, v.nanos)
            .ok_or(Error::invalid_argument("could not parse DateTimeValue into DateTime".into())),
        TypedValue::Integer(s) => {
            if let Some(dt) = DateTime::from_timestamp(s as i64, 0) {
                return Ok(dt);
            } else {
                Err(Error::invalid_argument("could not parse integer into DateTime".into()))
            }
        }
        TypedValue::String(s) => datetime_from_string(&s),
        _ => Err(Error::invalid_argument("cannot parse data type into DateTime".into())),
    }
}

fn datetime_from_string(s: &String) -> Result<DateTime<Utc>, Error> {
    match datetime_from_epoch(s) {
        Ok(dt) => Ok(dt),
        Err(_) => datetime_from_raw(s),
    }
}
