#![no_std]
#![no_main]
extern crate alloc;
use vstd::prelude::*;
mod echo;
mod read;
mod write;


use vrt::syscall::{
    sys_read,
    sys_write,
};

pub enum Input<'a> {
    File { file: File, path: &'a str },
    Source(Source),
}

impl Input<'_> {
    fn name(&self) -> &str {
        match self {
            Self::File { path, .. } => path,
            Self::Source(_) => "standard input",
        }
    }
}

pub struct Source {
    handle: HandleID,
}

pub struct Sink {
    handle: HandleID,
}

impl Read for Source {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        sys_read(self.handle, buf.as_mut_ptr(), buf.len(), usize::MAX).map_err(Error::from)
    }
}

impl Write for Sink {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> { sys_write(self.handle, buf.as_ptr(), buf.len(), usize::MAX).map_err(Error::from) }
}

impl Read for Input<'_> {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let result = match self {
            Self::File { file, .. } => file.read(buf),
            Self::Source(source) => source.read(buf),
        };
        result.map_err(|error| error.context(format_args!("cannot read '{}'", self.name())))
    }
}

pub fn input_from_path(path: Option<&str>) -> Result<Input<'_>, Error> {
    match path {
        None | Some("-") => Ok(Input::Source(Source { handle: env::source() })),
        Some(path) => File::open(&Path::new(path)).map(|file| Input::File { file, path }),
    }
}

#[vapp::main]
fn main(pkg: &ProcessInitPackage) -> Result<(), Error> {
    let args = env::args();

    let Some(command) = args.get(1) else {
        return Err(Error::invalid_argument("stream needs a command".into()));
    };

    let command_args = &args[2..];

    match command.as_str() {
        "echo" => echo::run(command_args)?,
        "read" => read::run(command_args)?,
        "head" => read::head(command_args)?,
        "tail" => read::tail(command_args)?,
        "write" => write::run(command_args)?,
        _ => Err(Error::invalid_argument("not a valid `stream` command".into()))?,
    }

    Ok(())
}
