#![no_std]
#![no_main]
extern crate alloc;

mod echo;
mod read;
mod wc;

use vespertine_rt::println;
use vespertine_rt::syscall::{sys_close, sys_read, sys_write};
use vespertine_abi::{HandleID, ProcessInitPackage};
use vespertine_std::fs::{File, Path};
use vespertine_std::{Error, Read, Write};
use vespertine_std::env;

pub enum Input {
    File(File),
    Source(Source)
}

pub struct Source {
    handle: HandleID,
}

pub struct Sink {
    handle: HandleID,
}

impl Read for Source {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        sys_read(self.handle, buf.as_mut_ptr(), buf.len(), usize::MAX)
            .map_err(Error::from)
    }
}

impl Write for Sink {
    fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        sys_write(self.handle, buf.as_ptr(), buf.len(), usize::MAX)
            .map_err(Error::from)
    }
}

impl Read for Input {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        match self {
            Input::File(file) => file.read(buf),
            Input::Source(source) => source.read(buf),
        }
    }
}

pub fn input_from_path(path: Option<&str>) -> Result<Input, Error> {
    match path {
        None | Some("-") => Ok(Input::Source(Source{ handle: env::source() })),
        Some(path) => File::open(&Path::new(path)).map(Input::File),
    }
}

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        println!("[ERROR] ns error: {:?}", e);
    }
    let _ = sys_close(env::sink());
}

fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
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
        "wc"   => wc::run(command_args)?,
        _ => Err(Error::invalid_argument("not a valid `stream` command".into()))?,
    }

    Ok(())
}

