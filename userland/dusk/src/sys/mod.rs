use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Display;

use vespertine_abi::app::hesper::AppIoMode;
use vespertine_abi::{
    HandleID,
    ProcessExitInfo,
};
use vespertine_rt::thread as rt_thread;
use vespertine_std::hesper::Launcher;
use vespertine_std::socket::Socket;
use vespertine_std::term::unset_raw_mode;
use vespertine_std::typed::render_typed_stream;
use vespertine_std::{
    Error,
    ErrorKind,
    HandleWriter,
    Process,
    env,
};

use crate::parser::ast::{
    BaseNode,
    CommandNode,
};
use crate::runtime::env::ShellContext;

#[derive(Debug, Clone)]
pub enum ShellResult {
    None,
    InternalError(Error),
    Launched(ProcessExitInfo),
    ChangeDirFail(String, Error),
    FailedToLaunch(String, Error),
    FailedToRender(String, Error),
    _AccessDenied(String),
    NotFound(String),
}

impl Display for ShellResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellResult::None => core::fmt::Result::Ok(()),
            ShellResult::Launched(ei) => write!(f, "exit info: {:?}", ei),
            ShellResult::InternalError(e) => write!(f, "internal error: {:?}", e),
            ShellResult::ChangeDirFail(path, err) => {
                write!(f, "couldn't change dir to {}: {:?}", path, err)
            }
            ShellResult::FailedToLaunch(name, err) => {
                write!(f, "failed to launch {}: {:?}", name, err)
            }
            ShellResult::FailedToRender(name, err) => {
                write!(f, "failed to render {}: {:?}", name, err)
            }
            ShellResult::_AccessDenied(name) => write!(f, "access denied: {}", name),
            ShellResult::NotFound(name) => write!(f, "not found: {}", name),
        }
    }
}

pub enum CommandSink {
    Terminal,
    Pipe,
}

pub enum SpawnedOutput {
    Terminal,
    Piped { kind: AppIoMode, socket: Socket },
}

pub struct SpawnedCommand {
    pub process: Process,
    pub output: SpawnedOutput,
}

#[derive(Debug, Clone, Copy)]
pub struct ProgramMetadata {
    input: AppIoMode,
    output: AppIoMode,
}

fn load_program_metadata(name: &str) -> Result<ProgramMetadata, ShellResult> {
    let mut launcher = Launcher::connect().map_err(|error| ShellResult::FailedToLaunch(name.into(), error))?;

    let response = launcher.metadata(name).map_err(|error| ShellResult::FailedToLaunch(name.into(), error))?;

    match response.status {
        HESPER_STATUS_OK => {
            if response.output == AppIoMode::Any {
                return Err(ShellResult::FailedToLaunch(
                    name.into(),
                    Error::invalid_argument("application manifest cannot declare output = any".into()),
                ));
            }

            Ok(ProgramMetadata { input: response.input, output: response.output })
        }
        HESPER_STATUS_NOT_FOUND => Err(ShellResult::NotFound(name.into())),
        HESPER_STATUS_INVALID_REQUEST => {
            Err(ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("application bundle contains an invalid manifest".into())))
        }
        _ => Err(ShellResult::FailedToLaunch(name.into(), Error::unknown("Hesper failed to return application metadata".into()))),
    }
}

pub fn launch_command(name: &str, args: &[String], context: &ShellContext) -> ShellResult {
    let metadata = match load_program_metadata(name) {
        Ok(metadata) => metadata,
        Err(result) => return result,
    };

    match metadata.output {
        AppIoMode::Direct | AppIoMode::Text => {
            let spawned = match spawn_command(name, args, context, env::source(), CommandSink::Terminal, metadata) {
                Ok(spawned) => spawned,
                Err(result) => return result,
            };
            wait_process(name, spawned.process)
        }
        AppIoMode::Typed => {
            let spawned = match spawn_command(name, args, context, env::source(), CommandSink::Pipe, metadata) {
                Ok(spawned) => spawned,
                Err(result) => return result,
            };

            let SpawnedOutput::Piped { socket, .. } = spawned.output else {
                return ShellResult::FailedToLaunch(
                    name.into(),
                    Error::invalid_argument("typed application did not produce a pipe".into()),
                );
            };

            let render_result = render_typed_stream(socket, HandleWriter::new(env::sink()));

            let wait_result = spawned.process.wait();
            let _ = unset_raw_mode();

            match (render_result, wait_result) {
                (Ok(()), Ok(exit)) => ShellResult::Launched(exit),
                (_, Err(error)) => ShellResult::FailedToLaunch(name.into(), error),
                (Err(error), _) => ShellResult::FailedToRender(name.into(), error),
            }
        }
        AppIoMode::Any => {
            ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("application has an invalid output mode".into()))
        }
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

pub fn spawn_command(
    name: &str, args: &[String], context: &ShellContext, source: HandleID, sink: CommandSink, metadata: ProgramMetadata,
) -> Result<SpawnedCommand, ShellResult> {
    if metadata.output == AppIoMode::Direct && !matches!(sink, CommandSink::Terminal) {
        return Err(ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("direct application cannot be piped".into())));
    }

    let mut launcher = Launcher::connect().map_err(|error| ShellResult::FailedToLaunch(name.into(), error))?;

    match sink {
        CommandSink::Terminal => {
            let process = launcher.launch(name, args, source, env::sink(), context.cwd_handle()).map_err(|error| {
                if error.kind == ErrorKind::NotFound {
                    ShellResult::NotFound(name.into())
                } else {
                    ShellResult::FailedToLaunch(name.into(), error)
                }
            })?;
            Ok(SpawnedCommand { process, output: SpawnedOutput::Terminal })
        }
        CommandSink::Pipe => {
            let (read_end, child_sink) = Socket::new_pair().map_err(|error| ShellResult::FailedToLaunch(name.into(), error))?;

            let process = launcher.launch(name, args, source, child_sink.handle(), context.cwd_handle()).map_err(|error| {
                if error.kind == ErrorKind::NotFound {
                    ShellResult::NotFound(name.into())
                } else {
                    ShellResult::FailedToLaunch(name.into(), error)
                }
            })?;

            drop(child_sink);

            Ok(SpawnedCommand { process, output: SpawnedOutput::Piped { kind: metadata.output, socket: read_end } })
        }
    }
}

struct PipelineRun {
    processes: Vec<(String, Process)>,
    output: SpawnedOutput,
    adapter_sockets: Vec<Socket>,
    adapter_threads: Vec<rt_thread::JoinHandle>,
}

pub fn launch_base(base: BaseNode, context: &ShellContext) -> ShellResult {
    let run = match spawn_base(&base, context, env::source(), CommandSink::Terminal) {
        Ok(run) => run,
        Err(result) => return result,
    };

    let render_result = match run.output {
        SpawnedOutput::Terminal => Ok(()),
        SpawnedOutput::Piped { kind: AppIoMode::Typed, socket } => render_typed_stream(socket, HandleWriter::new(env::sink())),
        SpawnedOutput::Piped { .. } => Err(Error::invalid_argument("final pipeline node must be terminal-renderable".into())),
    };

    let mut final_result = ShellResult::None;

    for (name, process) in run.processes {
        match process.wait() {
            Ok(exit) => final_result = ShellResult::Launched(exit),
            Err(error) => return ShellResult::FailedToLaunch(name, error),
        }
    }

    let _ = unset_raw_mode();

    match render_result {
        Ok(()) => final_result,
        Err(error) => ShellResult::FailedToRender("pipeline".into(), error),
    }
}

fn spawn_base(base: &BaseNode, context: &ShellContext, source: HandleID, sink: CommandSink) -> Result<PipelineRun, ShellResult> {
    match base {
        BaseNode::Cmd(cmd) => spawn_command_node(cmd, context, source, sink),

        BaseNode::Pipe(left, right) => {
            let left_run = spawn_base(left, context, source, CommandSink::Pipe)?;

            let SpawnedOutput::Piped { kind: left_kind, socket: pipe_source } = left_run.output else {
                return Err(ShellResult::FailedToLaunch(
                    "pipeline".into(),
                    Error::invalid_argument("left side of pipe did not produce a socket".into()),
                ));
            };

            let right_input = first_input_mode(right)?;
            check_pipe_compat(left_kind, right_input)?;

            let mut adapter_sockets = left_run.adapter_sockets;
            let mut adapter_threads = left_run.adapter_threads;

            let right_source = if left_kind == AppIoMode::Typed && right_input == Some(AppIoMode::Text) {
                let (text_rx, text_tx) = Socket::new_pair().map_err(|e| ShellResult::FailedToLaunch("pipeline".into(), e))?;

                let source_socket = pipe_source;
                let thread = rt_thread::spawn(move || {
                    let _ = render_typed_stream(source_socket, HandleWriter::new(text_tx.handle()));
                })
                .map_err(|e| ShellResult::FailedToLaunch("pipeline".into(), Error::from(e)))?;

                let handle = text_rx.handle();
                adapter_sockets.push(text_rx);
                adapter_threads.push(thread);
                handle
            } else {
                pipe_source.handle()
            };

            let right_run = spawn_base(right, context, right_source, sink)?;

            let mut processes = left_run.processes;
            processes.extend(right_run.processes);
            adapter_sockets.extend(right_run.adapter_sockets);
            adapter_threads.extend(right_run.adapter_threads);

            Ok(PipelineRun { processes, output: right_run.output, adapter_sockets, adapter_threads })
        }
    }
}

fn spawn_command_node(cmd: &CommandNode, context: &ShellContext, source: HandleID, sink: CommandSink) -> Result<PipelineRun, ShellResult> {
    let CommandNode::Run { exec, args } = cmd else {
        return Err(ShellResult::FailedToLaunch(
            "pipeline".into(),
            Error::invalid_argument("only external commands are supported in pipelines for now".into()),
        ));
    };

    let metadata = load_program_metadata(exec.as_str())?;

    let actual_sink = match sink {
        CommandSink::Pipe => CommandSink::Pipe,
        CommandSink::Terminal if metadata.output == AppIoMode::Typed => CommandSink::Pipe,
        CommandSink::Terminal => CommandSink::Terminal,
    };

    let spawned = spawn_command(exec.as_str(), args, context, source, actual_sink, metadata)?;

    Ok(PipelineRun {
        processes: vec![(exec.clone(), spawned.process)],
        output: spawned.output,
        adapter_sockets: Vec::new(),
        adapter_threads: Vec::new(),
    })
}

fn check_pipe_compat(left_output: AppIoMode, right_input: Option<AppIoMode>) -> Result<(), ShellResult> {
    let Some(input) = right_input else {
        return Ok(());
    };

    let accepted = match input {
        AppIoMode::Any => true,
        AppIoMode::Typed => left_output == AppIoMode::Typed,
        AppIoMode::Text => left_output == AppIoMode::Text || left_output == AppIoMode::Typed,
        AppIoMode::Direct => false,
    };

    if accepted {
        Ok(())
    } else {
        Err(ShellResult::FailedToLaunch("pipeline".into(), Error::invalid_argument("pipeline type mismatch".into())))
    }
}

fn first_input_mode(node: &BaseNode) -> Result<Option<AppIoMode>, ShellResult> {
    match node {
        BaseNode::Cmd(CommandNode::Run { exec, .. }) => Ok(Some(load_program_metadata(exec.as_str())?.input)),
        BaseNode::Pipe(left, _) => first_input_mode(left),
        _ => Ok(None),
    }
}
