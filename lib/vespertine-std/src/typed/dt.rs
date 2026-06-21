extern crate alloc;
use alloc::format;
use alloc::string::String;
use core::fmt::Display;

use vespertine_abi::typed::{
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
}
