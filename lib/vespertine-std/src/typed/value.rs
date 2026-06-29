extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Display;

use vespertine_abi::typed::{
    DateTimeValue,
    DateValue,
    FileSizeValue,
    TimeValue,
    USER_DISPLAY_NAME_MAX,
    USER_NAME_MAX,
    UserValue,
    ValueType,
};

use crate::Error;
use crate::typed::{
    DateTimeStyle,
    datetime_display,
};

#[derive(Debug, Clone, Copy)]
pub enum FileSizeStyle {
    Bytes,
    Iec,
    Si,
}

pub struct FileSizeDisplay {
    value: FileSizeValue,
    options: DisplayOptions,
}

impl FileSizeDisplay {
    pub fn new(value: FileSizeValue) -> Self { Self { value, options: DisplayOptions::default() } }

    pub fn options(mut self, options: DisplayOptions) -> Self {
        self.options = options;
        self
    }

    pub fn style(mut self, style: FileSizeStyle) -> Self {
        self.options.filesize_style = style;
        self
    }

    pub fn precision(mut self, precision: usize) -> Self {
        self.options.filesize_precision = precision;
        self
    }
}

impl Display for FileSizeDisplay {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str(&format_filesize(self.value, self.options)) }
}

impl Display for UserDisplay {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str(&format_user(self.value, self.options)) }
}

pub fn user_display(value: UserValue) -> UserDisplay { UserDisplay::new(value) }

pub fn user_value(id: u32) -> UserValue {
    UserValue {
        id,
        name_len: 0,
        display_name_len: 0,
        first_name_len: 0,
        last_name_len: 0,
        flags: 0,
        reserved: 0,
        name: [0; USER_NAME_MAX],
        display_name: [0; USER_DISPLAY_NAME_MAX],
        first_name: [0; USER_DISPLAY_NAME_MAX],
        last_name: [0; USER_DISPLAY_NAME_MAX],
    }
}

pub fn named_user_value(id: u32, name: &str, display_name: &str, first_name: &str, last_name: &str) -> Result<UserValue, Error> {
    if name.len() > USER_NAME_MAX {
        return Err(Error::name_too_long("user name is too long".into()));
    }
    if display_name.len() > USER_DISPLAY_NAME_MAX {
        return Err(Error::name_too_long("display name is too long".into()));
    }
    if first_name.len() > USER_DISPLAY_NAME_MAX {
        return Err(Error::name_too_long("first name is too long".into()));
    }
    if last_name.len() > USER_DISPLAY_NAME_MAX {
        return Err(Error::name_too_long("last name is too long".into()));
    }

    let mut value = user_value(id);
    value.name_len = name.len() as u8;
    value.display_name_len = display_name.len() as u8;
    value.first_name_len = first_name.len() as u8;
    value.last_name_len = last_name.len() as u8;
    value.name[..name.len()].copy_from_slice(name.as_bytes());
    value.display_name[..display_name.len()].copy_from_slice(display_name.as_bytes());
    value.first_name[..first_name.len()].copy_from_slice(first_name.as_bytes());
    value.last_name[..last_name.len()].copy_from_slice(last_name.as_bytes());
    Ok(value)
}

pub fn format_user(value: UserValue, opts: DisplayOptions) -> String {
    let text = if opts.user_show_username { user_name_text(&value) } else { user_display_text(&value) };

    if text.is_empty() { format!("{}", value.id) } else { text }
}

fn user_name_text(value: &UserValue) -> String {
    let len = value.name_len as usize;
    if len > value.name.len() {
        return String::new();
    }

    core::str::from_utf8(&value.name[..len]).map(String::from).unwrap_or_else(|_| String::new())
}

fn user_display_text(value: &UserValue) -> String {
    let len = value.display_name_len as usize;
    if len > value.display_name.len() {
        return String::new();
    }

    core::str::from_utf8(&value.display_name[..len]).map(String::from).unwrap_or_else(|_| String::new())
}

pub struct UserDisplay {
    value: UserValue,
    options: DisplayOptions,
}

impl UserDisplay {
    pub fn new(value: UserValue) -> Self { Self { value, options: DisplayOptions::default() } }

    pub fn options(mut self, options: DisplayOptions) -> Self {
        self.options = options;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DisplayOptions {
    pub filesize_style: FileSizeStyle,
    pub filesize_precision: usize,
    pub datetime_style: DateTimeStyle,
    pub datetime_show_tz: bool,
    pub datetime_show_subsec: bool,
    pub user_show_username: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            filesize_style: FileSizeStyle::Iec,
            filesize_precision: 1,
            datetime_style: DateTimeStyle::Standard,
            datetime_show_tz: true,
            datetime_show_subsec: false,
            user_show_username: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TypedValue {
    String(String),
    Integer(i128),
    Float(f64),
    Bool(bool),
    Date(DateValue),
    Time(TimeValue),
    DateTime(DateTimeValue),
    FileSize(FileSizeValue),
    List { element_type: ValueType, items: Vec<TypedValue> },
    Record { schema_id: u64, fields: Vec<TypedValue> },
    User(UserValue),
}

impl TypedValue {
    pub fn display_with(&self, opts: DisplayOptions) -> String {
        match self {
            TypedValue::String(v) => v.clone(),
            TypedValue::Integer(v) => format!("{}", v),
            TypedValue::Float(v) => format!("{}", v),
            TypedValue::Bool(v) => format!("{}", v),
            TypedValue::Date(v) => format!("{}-{}-{}", v.year, v.month, v.day),
            TypedValue::Time(v) => format!("{}:{}:{}", v.hour, v.minute, v.second),
            TypedValue::DateTime(v) => format!("{}", datetime_display(*v).options(opts)),
            TypedValue::FileSize(v) => format!("{}", filesize_display(*v).options(opts)),
            TypedValue::List { items, .. } => format!("[{} items]", items.len()),
            TypedValue::Record { .. } => format!("[record]"),
            TypedValue::User(v) => format!("{}", user_display(*v).options(opts)),
        }
    }
}

pub fn format_filesize(v: FileSizeValue, opts: DisplayOptions) -> String {
    if matches!(opts.filesize_style, FileSizeStyle::Bytes) {
        return format!("{} B", v.bytes);
    }

    let negative = v.bytes < 0;
    let bytes = if negative { (-v.bytes) as u128 } else { v.bytes as u128 };

    let text = match opts.filesize_style {
        FileSizeStyle::Bytes => unreachable!(),
        FileSizeStyle::Iec => format_scaled(bytes, 1024, &["B", "KiB", "MiB", "GiB", "TiB", "PiB"], opts.filesize_precision),
        FileSizeStyle::Si => format_scaled(bytes, 1000, &["B", "KB", "MB", "GB", "TB", "PB"], opts.filesize_precision),
    };

    if negative { format!("-{}", text) } else { text }
}

fn format_scaled(bytes: u128, base: u128, units: &[&str], precision: usize) -> String {
    let mut unit = 0usize;
    let mut scale = 1u128;

    while unit + 1 < units.len() && bytes >= scale * base {
        scale *= base;
        unit += 1;
    }

    if unit == 0 {
        return format!("{} {}", bytes, units[unit]);
    }

    let whole = bytes / scale;
    let rem = bytes % scale;

    if precision == 0 {
        return format!("{} {}", whole, units[unit]);
    }

    let factor = pow10(precision);
    let frac = (rem * factor) / scale;

    format!("{}.{:0width$} {}", whole, frac, units[unit], width = precision)
}

fn pow10(n: usize) -> u128 {
    let mut out = 1u128;
    for _ in 0..n {
        out *= 10;
    }
    out
}

pub fn filesize_display(value: FileSizeValue) -> FileSizeDisplay { FileSizeDisplay::new(value) }
