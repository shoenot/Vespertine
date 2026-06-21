use alloc::string::String;
use alloc::vec::Vec;

use vespertine_abi::typed::ValueType;
use vespertine_cli::args::{
    Command,
    Opt,
};
use vespertine_std::typed::{
    RecordStream,
    TypedValue,
    TypedWriter,
};
use vespertine_std::{
    Error,
    HandleWriter,
    Read,
};

use crate::input_from_path;

static READ_OPTIONS: &[Opt] = &[
    Opt::flag("number", Some('n'), Some("number")),
    Opt::flag("squeeze-blank", Some('s'), Some("squeeze-blank")),
    Opt::flag("help", Some('h'), Some("help")),
];

static TRUNCATED_READ_OPTIONS: &[Opt] = &[
    Opt::value("lines", Some('n'), Some("num-lines")),
    Opt::flag("number", Some('N'), Some("number")),
    Opt::flag("help", Some('h'), Some("help")),
];

enum LineOutput {
    Plain(TypedWriter<HandleWriter>),
    Numbered(RecordStream<HandleWriter>),
}

const STREAM_LINE_SCHEMA: u64 = 2;

impl LineOutput {
    fn new(numbered: bool) -> Result<Self, Error> {
        if numbered {
            let out = RecordStream::typed_default_out(
                STREAM_LINE_SCHEMA,
                &[("number", ValueType::Integer), ("line", ValueType::String)],
                &["number", "line"],
            )?;
            out.table(&["number", "line"])?;
            Ok(Self::Numbered(out))
        } else {
            Ok(Self::Plain(TypedWriter::out()))
        }
    }

    fn line(&mut self, number: usize, bytes: &[u8]) -> Result<(), Error> {
        let text = str::from_utf8(bytes).map_err(|_| Error::invalid_encoding("input contains invalid UTF-8".into()))?;

        match self {
            Self::Plain(out) => out.value(&TypedValue::String(String::from(text))),
            Self::Numbered(out) => out.row_values(&[TypedValue::Integer(number as i128), TypedValue::String(String::from(text))]),
        }
    }

    fn finish(&mut self) -> Result<(), Error> {
        match self {
            Self::Plain(out) => out.stream_end(),
            Self::Numbered(out) => out.finish(),
        }
    }
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("read").options(READ_OPTIONS).parse(args).map_err(Error::from)?;

    if matches.flag("help") {
        return Ok(());
    }

    if matches.positional_count() > 1 {
        return Err(Error::invalid_argument(
            "usage: stream read [flags] [file | - ]\n
                                            flags:\n
                                            \t-n, --number:        print line numbers\n
                                            \t-s, --squeeze-blank: squeeze multiple consecutive blank lines\n
                                            \t-h, --help:          print this help text"
                .into(),
        ));
    }

    let input = input_from_path(matches.positional(0))?;
    copy_lines(&input, matches.flag("number"), matches.flag("squeeze-blank"))
}

pub fn head(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("head").options(TRUNCATED_READ_OPTIONS).parse(args).map_err(Error::from)?;

    if matches.flag("help") {
        return Ok(());
    }

    if matches.positional_count() > 1 {
        return Err(Error::invalid_argument(
            "usage: stream head [flags] [file | - ]\n
                                            flags:\n
                                            \t-n, --num-lines: number of lines to print (from the beginning)\n
                                            \t-N, --numbers:   print line numbers\n
                                            \t-h, --help:      print this help text"
                .into(),
        ));
    }

    let input = input_from_path(matches.positional(0))?;
    copy_head(&input, matches.value("lines"), matches.flag("number"))
}

pub fn tail(args: &[String]) -> Result<(), Error> {
    let matches = Command::new("tail").options(TRUNCATED_READ_OPTIONS).parse(args).map_err(Error::from)?;

    if matches.flag("help") {
        return Ok(());
    }

    if matches.positional_count() > 1 {
        return Err(Error::invalid_argument(
            "usage: stream tail [flags] [file | - ]\n
                                            flags:\n
                                            \t-n, --num-lines: number of lines to print (from the beginning)\n
                                            \t-N, --numbers:   print line numbers\n
                                            \t-h, --help:      print this help text"
                .into(),
        ));
    }

    let input = input_from_path(matches.positional(0))?;
    copy_tail(&input, matches.value("lines"), matches.flag("number"))
}

fn parse_line_limit(raw: Option<&str>) -> Result<usize, Error> {
    match raw {
        Some(nstr) => nstr.parse::<usize>().map_err(|_| Error::invalid_argument("-n argument must be a number".into())),
        None => Ok(10),
    }
}

fn trim_cr(line: &mut Vec<u8>) {
    if line.ends_with(b"\r") {
        line.pop();
    }
}

fn copy_lines<R: Read>(input: &R, numbered: bool, squeeze: bool) -> Result<(), Error> {
    let mut out = LineOutput::new(numbered)?;
    let mut buf = [0u8; 4096];
    let mut line = Vec::new();
    let mut line_no = 1usize;
    let mut last_blank = false;

    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }

        for &b in &buf[..n] {
            if b == b'\n' {
                if line.ends_with(b"\r") {
                    line.pop();
                }

                let blank = line.is_empty();
                if !(squeeze && blank && last_blank) {
                    out.line(line_no, &line)?;
                    line_no += 1;
                }

                last_blank = blank;
                line.clear();
            } else {
                line.push(b);
            }
        }
    }

    if !line.is_empty() {
        out.line(line_no, &line)?;
    }

    out.finish()
}

fn copy_head<R: Read>(input: &R, num_lines: Option<&str>, numbered: bool) -> Result<(), Error> {
    let limit = parse_line_limit(num_lines)?;
    let mut out = LineOutput::new(numbered)?;

    if limit == 0 {
        return out.finish();
    }

    let mut buf = [0u8; 4096];
    let mut line = Vec::new();
    let mut line_no = 1usize;
    let mut emitted = 0usize;

    while emitted < limit {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }

        for &b in &buf[..n] {
            if b == b'\n' {
                trim_cr(&mut line);
                out.line(line_no, &line)?;

                line.clear();
                line_no += 1;
                emitted += 1;

                if emitted >= limit {
                    return out.finish();
                }
            } else {
                line.push(b);
            }
        }
    }

    if emitted < limit && !line.is_empty() {
        trim_cr(&mut line);
        out.line(line_no, &line)?;
    }

    out.finish()
}

fn copy_tail<R: Read>(input: &R, num_lines: Option<&str>, numbered: bool) -> Result<(), Error> {
    let limit = parse_line_limit(num_lines)?;
    let mut out = LineOutput::new(numbered)?;

    if limit == 0 {
        return out.finish();
    }

    let mut buf = [0u8; 4096];
    let mut line = Vec::new();
    let mut lines: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut line_no = 1usize;

    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }

        for &b in &buf[..n] {
            if b == b'\n' {
                trim_cr(&mut line);
                lines.push((line_no, core::mem::take(&mut line)));
                line_no += 1;
            } else {
                line.push(b);
            }
        }
    }

    if !line.is_empty() {
        trim_cr(&mut line);
        lines.push((line_no, line));
    }

    let start = lines.len().saturating_sub(limit);

    for (number, line) in &lines[start..] {
        out.line(*number, line)?;
    }

    out.finish()
}
