use alloc::string::String;

use vespertine_abi::app::hesper::AppIoMode;
use vespertine_abi::HandleID;
use vespertine_std::hesper::Launcher;
use vespertine_std::socket::Socket;
use vespertine_std::term::unset_raw_mode;
use vespertine_std::typed::render_typed_stream;
use vespertine_std::{
    env,
    Error,
    ErrorKind,
    HandleWriter,
    Process,
};

use crate::runtime::env::ShellContext;
use crate::sys::metadata::{
    load_program_metadata,
    ProgramMetadata,
};
use crate::sys::mode::{
    choose_mode,
    needs_empty_direct_source,
    CommandSink,
};
use crate::sys::ShellResult;

pub(super) enum SpawnedOutput {
    Terminal,
    Piped { kind: AppIoMode, socket: Socket },
}

pub(super) struct SpawnedCommand {
    pub process: Process,
    pub output: SpawnedOutput,
}

pub fn launch_command(name: &str, args: &[String], context: &ShellContext) -> ShellResult {
    let metadata = match load_program_metadata(name) {
        Ok(metadata) => metadata,
        Err(result) => return result,
    };

    let mode = match choose_mode(name, args, metadata, CommandSink::Terminal) {
        Ok(mode) => mode,
        Err(result) => return result,
    };

    let empty_source = if needs_empty_direct_source(metadata.input, mode) {
        match Socket::new_pair() {
            Ok((read_end, write_end)) => {
                drop(write_end);
                Some(read_end)
            },
            Err(error) => return ShellResult::FailedToLaunch(name.into(), error),
        }
    } else {
        None
    };

    let source = empty_source.as_ref().map(|socket| socket.handle()).unwrap_or_else(env::source);

    match mode {
        AppIoMode::Terminal | AppIoMode::Text => {
            let spawned = match spawn_command(name, args, context, source, CommandSink::Terminal, metadata) {
                Ok(spawned) => spawned,
                Err(result) => return result,
            };
            if let Err(error) = spawned.process.resume() {
                return ShellResult::FailedToLaunch(name.into(), error);
            }
            wait_process(name, spawned.process)
        },
        AppIoMode::Typed => {
            let spawned = match spawn_command(name, args, context, source, CommandSink::Pipe, metadata) {
                Ok(spawned) => spawned,
                Err(result) => return result,
            };

            let SpawnedOutput::Piped { socket, .. } = spawned.output else {
                return ShellResult::FailedToLaunch(
                    name.into(),
                    Error::invalid_argument("typed application did not produce a pipe".into()),
                );
            };

            if let Err(error) = spawned.process.resume() {
                return ShellResult::FailedToLaunch(name.into(), error);
            }
            
            let render_result = render_typed_stream(socket, HandleWriter::new(env::sink()));
            let wait_result = spawned.process.wait();
            let _ = unset_raw_mode();

            match (render_result, wait_result) {
                (Ok(()), Ok(exit)) => ShellResult::Launched(exit),
                (_, Err(error)) => ShellResult::FailedToLaunch(name.into(), error),
                (Err(error), _) => ShellResult::FailedToRender(name.into(), error),
            }
        },
        AppIoMode::Any => {
            ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("application selected invalid launch mode".into()))
        },
    }
}

pub(super) fn spawn_command(
    name: &str, args: &[String], context: &ShellContext, source: HandleID, sink: CommandSink, metadata: ProgramMetadata,
) -> Result<SpawnedCommand, ShellResult> {
    let mode = choose_mode(name, args, metadata, sink)?;
    if mode == AppIoMode::Terminal && !matches!(sink, CommandSink::Terminal) {
        return Err(ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("terminal application cannot be piped".into())));
    }

    let mut launcher = Launcher::connect().map_err(|error| ShellResult::FailedToLaunch(name.into(), error))?;

    match sink {
        CommandSink::Terminal => {
            let process = launcher.launch(name, args, mode, source, env::sink(), context.cwd_handle()).map_err(map_launch_error(name))?;
            Ok(SpawnedCommand { process, output: SpawnedOutput::Terminal })
        },
        CommandSink::Pipe => {
            let (read_end, child_sink) = Socket::new_pair().map_err(|error| ShellResult::FailedToLaunch(name.into(), error))?;
            let process = launcher.launch(name, args, mode, source, child_sink.handle(), context.cwd_handle()).map_err(map_launch_error(name))?;

            drop(child_sink);

            Ok(SpawnedCommand { process, output: SpawnedOutput::Piped { kind: mode, socket: read_end } })
        },
    }
}

fn wait_process(name: &str, proc: Process) -> ShellResult {
    let res = match proc.wait() {
        Ok(exit) => ShellResult::Launched(exit),
        Err(e) => ShellResult::FailedToLaunch(name.into(), e),
    };

    let _ = unset_raw_mode();
    res
}

fn map_launch_error(name: &str) -> impl FnOnce(Error) -> ShellResult + '_ {
    move |error| {
        if error.kind == ErrorKind::NotFound {
            ShellResult::NotFound(name.into())
        } else {
            ShellResult::FailedToLaunch(name.into(), error)
        }
    }
}
