#![no_std]
#![no_main]

use chrono::DateTime;
use chrono_tz::Tz;
use vespertine_abi::{AccessRights, ProcessInitPackage};
use vespertine_rt::{println, syscall::sys_close};
use vespertine_std::{Error, ErrorKind, clock::{Clock, Time}, env};

extern crate alloc;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] dt error: {:?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let (ts, _) = Time::now();
    let dt = DateTime::from_timestamp_secs(ts as i64)
        .ok_or(0)
        .map_err(|_| Error {
            kind: ErrorKind::Unknown,
            message: "Error converting timestamp to dt".into(),
        })?;

    let args = env::args();

    if args.len() < 2 {
        println!("{}", dt);
    } else {
        match args[1].as_str() {
            "timestamp" => {
                println!("{}", ts);
            }
            "tz" => {
                if args.len() < 3 {
                    return Err(Error {
                        kind: ErrorKind::InvalidArgument,
                        message: "The `tz` option requires a timezone argument".into(),
                    });
                } else {
                    let tz: Tz = args[2].parse().map_err(|_| Error {
                        kind: ErrorKind::InvalidArgument,
                        message: "Invalid timezone".into(),
                    })?;
                    let dt_tz: DateTime<Tz> = dt.with_timezone(&tz);
                    println!("{}", dt_tz);
                }
            }
            "from" => {
                let ts: i64 = args[2].parse().map_err(|_| Error {
                    kind: ErrorKind::InvalidArgument,
                    message: "Invalid timestamp".into(),
                })?;
                let dt = DateTime::from_timestamp_secs(ts as i64)
                    .ok_or(0)
                    .map_err(|_| Error {
                        kind: ErrorKind::Unknown,
                        message: "Error converting timestamp to dt".into(),
                    })?;
                println!("{}", dt);
            }
            _ => {
                return Err(Error {
                    kind: ErrorKind::InvalidArgument,
                    message: "Invalid Operation".into(),
                });
            }
        }
    }
    Ok(())
}
