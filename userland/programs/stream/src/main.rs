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
fn main(_pkg: &ProcessInitPackage) -> Result<(), Error> {
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
        "int-test" => interrupt_test(command_args)?,
        _ => Err(Error::invalid_argument("not a valid `stream` command".into()))?,
    }

    Ok(())
}

fn interrupt_test(args: &[String]) -> Result<(), Error> {
    use vespertine_abi::ProcSignal;
    use vrt::syscall::sys_send_signal;
    use vstd::notifications::Notifications;

    let [mode] = args else {
        return Err(Error::invalid_argument(
            "usage: stream interrupt-test handled|discard|unhandled".into(),
        ));
    };

    let me = env::self_handle();
    let out = TypedWriter::out();

    let message = match mode.as_str() {
        "handled" => {
            let inbox = Notifications::claim()?;
            sys_send_signal(me, ProcSignal::Interrupt)?;

            let sender = inbox.recv()?;
            let expected = vstd::Process::info_from_handle(me)?.pid;
            if sender != expected {
                return Err(Error::invalid_argument(
                    format!("expected sender {expected}, received {sender}"),
                ));
            }

            format!("handled interrupt from pid {sender}")
        }
        "discard" => {
            let inbox = Notifications::claim()?;
            sys_send_signal(me, ProcSignal::Interrupt)?;
            drop(inbox); // Deliberately leave the interrupt unread.

            let next = Notifications::claim()?;
            if next.try_take()?.is_some() {
                return Err(Error::invalid_argument(
                    "old interrupt appeared in the new claim".into(),
                ));
            }

            "survived release; new claim is empty".into()
        }
        "unhandled" => {
            // Give Dusk a complete typed stream before we terminate.
            out.stream_end()?;
            sys_send_signal(me, ProcSignal::Interrupt)?;
            return Err(Error::invalid_argument(
                "process continued after an unhandled interrupt".into(),
            ));
        }
        "wake" => {
            let inbox = Notifications::claim()?;

            // Dusk consumes this byte before handing the socket to the typed renderer.
            vstd::HandleWriter::new(env::sink()).write_all(b"R")?;

            let sender = inbox.recv()?;
            let me = vstd::Process::info_from_handle(me)?.pid;
            if sender == me {
                return Err(Error::invalid_argument(
                    "expected an interrupt from another process".into(),
                ));
            }

            format!("woke pid {me} with interrupt from pid {sender}")
        }
        _ => return Err(Error::invalid_argument("unknown test mode".into())),
    };

    out.value(&TypedValue::String(message))?;
    out.stream_end()
}
