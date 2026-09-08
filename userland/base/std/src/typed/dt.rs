extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Display;

use vespertine_abi::typed::{
    DATETIME_EMPTY,
    DATETIME_HAS_OFFSET,
    DateTimeValue,
    DateValue,
    TimeValue,
};
use vespertine_common::datetime::{
    datetime_to_epoch,
    epoch_to_datetime,
};

use crate::typed::DisplayOptions;

#[derive(Debug, Clone, Copy)]
pub enum DateTimeStyle {
    Standard,
    StandardUS,
    Iso,
    Unix,
    Date,
    Time,
}

pub struct DateTimeDisplay {
    value: DateTimeValue,
    options: DisplayOptions,
}

impl DateTimeDisplay {
    pub fn new(value: DateTimeValue) -> Self { Self { value, options: DisplayOptions::default() } }

    pub fn style(mut self, style: DateTimeStyle) -> Self {
        self.options.datetime_style = style;
        self
    }

    pub fn show_tz(mut self, tz: bool) -> Self {
        self.options.datetime_show_tz = tz;
        self
    }

    pub fn show_subsec(mut self, subsec: bool) -> Self {
        self.options.datetime_show_subsec = subsec;
        self
    }

    pub fn options(mut self, options: DisplayOptions) -> Self {
        self.options = options;
        self
    }
}

impl Display for DateTimeDisplay {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str(&format_datetime(self.value, self.options)) }
}

pub fn format_datetime(v: DateTimeValue, opts: DisplayOptions) -> String {
    let offset_seconds = v.offset_minutes * 60;
    let adjusted = v.unix_seconds + offset_seconds as i64;

    let dt = epoch_to_datetime(adjusted);

    match opts.datetime_style {
        DateTimeStyle::Standard => format!("{:02}/{:02}/{:04}, {:02}:{:02}", dt.day, dt.month, dt.year, dt.hour, dt.minute),
        DateTimeStyle::StandardUS => format!("{:02}/{:02}/{:04}, {:02}:{:02}", dt.month, dt.day, dt.year, dt.hour, dt.minute),
        DateTimeStyle::Unix => format!("{}", v.unix_seconds),
        DateTimeStyle::Date => format!("{:04}-{:02}-{:02}", dt.year, dt.month, dt.day),
        DateTimeStyle::Time => format_time(&dt, v.nanos, opts),
        DateTimeStyle::Iso => {
            let tz =
                if opts.datetime_show_tz && (v.flags & DATETIME_HAS_OFFSET) != 0 { format_tz(v.offset_minutes) } else { String::new() };
            format!("{:04}-{:02}-{:02}T{}{}", dt.year, dt.month, dt.day, format_time(&dt, v.nanos, opts), tz)
        }
    }
}

fn format_time(dt: &vespertine_common::datetime::DateTime, nanos: u32, opts: DisplayOptions) -> String {
    if opts.datetime_show_subsec && nanos != 0 {
        format!("{:02}:{:02}:{:02}.{:09}", dt.hour, dt.minute, dt.second, nanos)
    } else {
        format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second)
    }
}

fn format_tz(offset_minutes: i32) -> String {
    if offset_minutes == 0 {
        return String::from("Z");
    }

    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let abs = offset_minutes.abs();
    format!("{}{:02}:{:02}", sign, abs / 60, abs % 60)
}

pub fn datetime_display(value: DateTimeValue) -> DateTimeDisplay { DateTimeDisplay::new(value) }

pub trait DateTimeValueExt {
    fn date(self) -> DateValue;
    fn time(self) -> TimeValue;
    fn from_epoch(seconds: i64, nanos: u32) -> DateTimeValue;
    fn from_date_and_time(date: DateValue, time: TimeValue) -> DateTimeValue;
    fn from_iso_string(iso: &str) -> Option<DateTimeValue>;
    fn as_iso_string(self) -> String;
}

impl DateTimeValueExt for DateTimeValue {
    fn date(self) -> DateValue {
        let dt = epoch_to_datetime(self.unix_seconds);
        DateValue { year: dt.year, month: dt.month as u8, day: dt.day as u8, calendar: 0, flags: 0 }
    }

    fn time(self) -> TimeValue {
        let dt = epoch_to_datetime(self.unix_seconds);
        TimeValue {
            hour: dt.hour as u8,
            minute: dt.minute as u8,
            second: dt.second as u8,
            reserved: 0,
            nanos: 0,
            offset_minutes: 0,
            flags: 0,
        }
    }

    fn from_epoch(seconds: i64, nanos: u32) -> DateTimeValue {
        DateTimeValue { unix_seconds: seconds, nanos, offset_minutes: 0, flags: DATETIME_HAS_OFFSET, calendar: 0, reserved: 0 }
    }

    fn from_date_and_time(date: DateValue, time: TimeValue) -> DateTimeValue {
        let dt = vespertine_common::datetime::DateTime {
            year: date.year,
            month: date.month as u32,
            day: date.day as u32,
            hour: time.hour as u32,
            minute: time.minute as u32,
            second: time.second as u32,
        };
        DateTimeValue {
            unix_seconds: datetime_to_epoch(dt),
            nanos: 0,
            offset_minutes: 0,
            flags: DATETIME_HAS_OFFSET,
            calendar: 0,
            reserved: 0,
        }
    }

    fn from_iso_string(iso: &str) -> Option<DateTimeValue> {
        let bytes = iso.as_bytes();
        if bytes.len() < 19 {
            return None;
        } // YYYY-MM-DDTHH:MM:SS at minimum

        // date
        if bytes[4] != b'-' || bytes[7] != b'-' || (bytes[10] != b'T' && bytes[10] != b' ') {
            return None;
        }

        let year = parse_digits(&bytes[0..4])? as i32;
        let month = parse_digits(&bytes[5..7])? as u8;
        let day = parse_digits(&bytes[8..10])? as u8;

        if bytes[13] != b':' || bytes[16] != b':' {
            return None;
        }

        // time
        let hour = parse_digits(&bytes[11..13])? as u8;
        let minute = parse_digits(&bytes[14..16])? as u8;
        let second = parse_digits(&bytes[17..19])? as u8;

        // validate
        if month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || minute > 59 || second > 60 {
            return None;
        }

        // optional fractional seconds
        let mut cursor = 19;
        let mut nanos = 0u32;

        if cursor < bytes.len() && bytes[cursor] == b'.' {
            cursor += 1;
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            let digit_count = cursor - start;
            if digit_count == 0 {
                return None;
            }

            let mut fraction = parse_digits(&bytes[start..cursor])?;
            // scale to nanoseconds (9 digits)
            if digit_count < 9 {
                fraction *= 10u32.pow((9 - digit_count) as u32);
            } else if digit_count > 9 {
                fraction /= 10u32.pow((digit_count - 9) as u32);
            }
            nanos = fraction;
        }

        // tz offset
        if cursor >= bytes.len() {
            return None;
        } // no tz indicator

        let offset_minutes = match bytes[cursor] {
            b'Z' | b'z' => {
                if cursor + 1 != bytes.len() {
                    return None;
                } // Z but its not the last letter
                0
            }
            sign @ (b'+' | b'-') => {
                let tz_bytes = &bytes[cursor + 1..];
                if tz_bytes.len() != 5 || tz_bytes[2] != b':' {
                    return None;
                }

                let tz_hour = parse_digits(&tz_bytes[0..2])? as i32;
                let tz_min = parse_digits(&tz_bytes[3..5])? as i32;
                if tz_hour > 23 || tz_min > 59 {
                    return None;
                }

                let total_minutes = tz_hour * 60 + tz_min;
                if sign == b'-' { -total_minutes } else { total_minutes }
            }
            _ => return None, // invalid timezone symbol
        };

        let date = DateValue { year, month, day, calendar: 0, flags: 0 };
        let time = TimeValue { hour, minute, second, nanos, offset_minutes, reserved: 0, flags: DATETIME_HAS_OFFSET };

        Some(DateTimeValue::from_date_and_time(date, time))
    }

    fn as_iso_string(self) -> String {
        let mut buf = Vec::with_capacity(35);
        let (date, time) = (self.date(), self.time());

        // date
        write_digits(&mut buf, date.year.abs() as u32, 4);
        buf.push(b'-');
        write_digits(&mut buf, date.month as u32, 2);
        buf.push(b'-');
        write_digits(&mut buf, date.day as u32, 2);

        buf.push(b'T');

        // time
        write_digits(&mut buf, time.hour as u32, 2);
        buf.push(b':');
        write_digits(&mut buf, time.minute as u32, 2);
        buf.push(b':');
        write_digits(&mut buf, time.second as u32, 2);

        // nanos
        if self.nanos > 0 {
            buf.push(b'.');
            write_digits(&mut buf, self.nanos, 9);
        }

        // offset
        if self.offset_minutes == 0 {
            buf.push(b'Z');
        } else {
            let abs_minutes = self.offset_minutes.abs();
            let tz_hour = abs_minutes / 60;
            let tz_min = abs_minutes % 60;

            if self.offset_minutes < 0 {
                buf.push(b'-');
            } else {
                buf.push(b'+');
            }

            write_digits(&mut buf, tz_hour as u32, 2);
            buf.push(b':');
            write_digits(&mut buf, tz_min as u32, 2);
        }

        unsafe { String::from_utf8_unchecked(buf) }
    }
}

pub fn convert_iso_datetime(iso: String) -> DateTimeValue {
    match DateTimeValue::from_iso_string(iso.as_str()) {
        Some(dt) => dt,
        None => DateTimeValue { unix_seconds: 0, nanos: 0, offset_minutes: 0, flags: DATETIME_EMPTY, reserved: 0, calendar: 0 },
    }
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    let mut val = 0u32;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

fn write_digits(buf: &mut alloc::vec::Vec<u8>, mut val: u32, width: usize) {
    let start_idx = buf.len();

    // write digits backwards
    for _ in 0..width {
        let digit = (val % 10) as u8;
        buf.push(b'0' + digit);
        val /= 10;
    }

    // reverse the newly appended slice to match correct order
    buf[start_idx..].reverse();
}
