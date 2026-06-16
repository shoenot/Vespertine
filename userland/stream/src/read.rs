use alloc::{format, string::String, vec::Vec};
use vespertine_cli::args::{Command, Opt};
use vespertine_std::{Error, Read, Write, env};

use crate::{Sink, input_from_path};

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

pub fn run(args: &[String]) -> Result<(), Error> { 
    let matches = Command::new("read")
        .options(READ_OPTIONS)
        .parse(args)
        .map_err(Error::from)?;

    if matches.flag("help") {
        return Ok(());
    }

    if matches.positional_count() > 1 {
        return Err(Error::invalid_argument("usage: stream read [flags] [file | - ]\n
                                            flags:\n
                                            \t-n, --number:        print line numbers\n
                                            \t-s, --squeeze-blank: squeeze multiple consecutive blank lines\n
                                            \t-h, --help:          print this help text".into()));
    }

    let input = input_from_path(matches.positional(0))?;
    let output = Sink { handle: env::sink() };

    copy_special(&input, &output, matches.flag("number"), matches.flag("squeeze-blank"))
}

pub fn head(args: &[String]) -> Result<(), Error> { 
    let matches = Command::new("head")
        .options(TRUNCATED_READ_OPTIONS)
        .parse(args)
        .map_err(Error::from)?;

    if matches.flag("help") {
        return Ok(());
    }

    if matches.positional_count() > 1 {
        return Err(Error::invalid_argument("usage: stream head [flags] [file | - ]\n
                                            flags:\n
                                            \t-n, --num-lines: number of lines to print (from the beginning)\n
                                            \t-N, --numbers:   print line numbers\n
                                            \t-h, --help:      print this help text".into()));
    }

    let input = input_from_path(matches.positional(0))?;
    let output = Sink { handle: env::sink() };
    let num_lines = matches.value("lines");

    copy_head(&input, &output, num_lines, matches.flag("number"))
}

pub fn tail(args: &[String]) -> Result<(), Error> { 
    let matches = Command::new("head")
        .options(TRUNCATED_READ_OPTIONS)
        .parse(args)
        .map_err(Error::from)?;

    if matches.flag("help") {
        return Ok(());
    }

    if matches.positional_count() > 1 {
        return Err(Error::invalid_argument("usage: stream tail [flags] [file | - ]\n
                                            flags:\n
                                            \t-n, --num-lines: number of lines to print (from the beginning)\n
                                            \t-N, --numbers:   print line numbers\n
                                            \t-h, --help:      print this help text".into()));
    }

    let input = input_from_path(matches.positional(0))?;
    let output = Sink { handle: env::sink() };
    let num_lines = matches.value("lines");

    copy_tail(&input, &output, num_lines, matches.flag("number"))
 }

fn copy_raw<R: Read, W: Write>(input: &R, output: &W) -> Result<(), Error> {
    let mut buf = [0u8; 4096];

    loop {
        let n = input.read(&mut buf)?;
        if n == 0 { return Ok(()) };
        output.write_all(&buf[..n])?;
    }
}

fn copy_special<R: Read, W: Write>(input: &R, output: &W, numbers: bool, squeeze: bool) -> Result<(), Error> {
    let mut buf = [0u8; 4096];
    let mut line = 1usize;
    let mut last_line_blank = false;
    let mut at_line_start = true;

    loop {
        let n = input.read(&mut buf)?;
        if n == 0 { return Ok(()) };
        for &b in &buf[..n] {
            let current_line_blank = at_line_start && b == b'\n';
            
            if squeeze && current_line_blank && last_line_blank {
                continue;
            }

            if at_line_start {
                if numbers {
                    let prefix = format!("{} ", line);
                    output.write_all(prefix.as_bytes())?;
                }

                at_line_start = false;
            }

            output.write_all(&[b])?;

            if b == b'\n' {
                line += 1;
                at_line_start = true;
                last_line_blank = current_line_blank;
            } else {
                last_line_blank = false;
            }
        }
    }
}

fn copy_head<R: Read, W: Write>(input: &R, output: &W, num_lines: Option<&str>, numbers: bool) -> Result<(), Error> {
    let mut buf = [0u8; 4096];
    let mut line = 1usize;
    let mut at_line_start = true;

    let limit = match num_lines {
        Some(nstr) => nstr
            .parse::<usize>()
            .map_err(|_| Error::invalid_argument("-n argument must be a number".into()))?,
        None => 10,
    };

    if limit == 0 { return Ok(()) };

    while line <= limit {
        let n = input.read(&mut buf)?;
        if n == 0 { return Ok(()) };
        for &b in &buf[..n] {
            if at_line_start {
                if line > limit {
                    return Ok(());
                }

                if numbers {
                    let prefix = format!("{} ", line);
                    output.write_all(prefix.as_bytes())?;
                }

                at_line_start = false;
            }

            output.write_all(&[b])?;

            if b == b'\n' {
                line += 1;
                at_line_start = true;
            }
        }
    }
    Ok(())
}

fn copy_tail<R: Read, W: Write>(input: &R, output: &W, num_lines: Option<&str>, numbers: bool) -> Result<(), Error> {
    let mut buf = [0u8; 4096];
    let mut line = 1usize;
    let mut at_line_start = true;

    let limit = match num_lines {
        Some(nstr) => nstr
            .parse::<usize>()
            .map_err(|_| Error::invalid_argument("-n argument must be a number".into()))?,
        None => 10,
    };

    if limit == 0 { return Ok(()) };

    let mut output_buffer = Vec::new();

    loop {
        let n = input.read(&mut buf)?;
        if n == 0 { break; }
        let mut cur = String::new();
        for &b in &buf[..n] {
            if at_line_start {
                if numbers {
                    let prefix = format!("{} ", line);
                    output.write_all(prefix.as_bytes())?;
                }

                at_line_start = false;
            }

            let cb = &[b].clone();
            let s = str::from_utf8(cb)
                .map_err(|_| Error::invalid_encoding("File contains invalid UTF-8".into()))?;
            cur.push_str(s);

            if b == b'\n' {
                line += 1;
                output_buffer.push(cur);
                cur = String::new();
                at_line_start = true;
            }
        }
    }

    let start_idx = output_buffer.len().saturating_sub(limit);
    let last_n = &output_buffer[start_idx..];
    for line in last_n {
        output.write_string(line.clone())?
    }
    Ok(())
}
