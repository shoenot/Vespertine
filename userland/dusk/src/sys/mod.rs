use alloc::string::String;
use alloc::vec::Vec;
use alloc::{
    format,
    vec,
};
use core::fmt::Display;

use vespertine_abi::tag::CAP_APP_TERMCTRL;
use vespertine_abi::{
    AccessRights,
    CapabilityID,
    HandleID,
    ProcessExitInfo,
};
use vespertine_rt::thread as rt_thread;
use vespertine_std::fs::{
    File,
    Path,
    PathBuf,
};
use vespertine_std::socket::Socket;
use vespertine_std::term::unset_raw_mode;
use vespertine_std::typed::render_typed_stream;
use vespertine_std::{
    Error,
    ErrorKind,
    Exec,
    HandleWriter,
    Process,
    Read,
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
    Piped { kind: ProgramOutput, socket: Socket },
}

pub struct SpawnedCommand {
    pub process: Process,
    pub output: SpawnedOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramInput {
    Any,
    Text,
    Typed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramOutput {
    Direct,
    Typed,
    Text,
}

pub struct ProgramManifest {
    pub input: ProgramInput,
    pub output: ProgramOutput,
    pub grants: Vec<(CapabilityID, AccessRights)>,
}

impl ProgramManifest {
    fn new() -> Self { Self { input: ProgramInput::Text, output: ProgramOutput::Text, grants: Vec::new() } }
}

impl Default for ProgramManifest {
    fn default() -> Self { Self::new() }
}

pub fn parse_manifest(p: &Path) -> Option<ProgramManifest> {
    let mf = File::open(p).ok()?;
    let contents = mf.read_to_string().ok()?;

    let mut manifest = ProgramManifest::default();

    for line in contents.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };

        match k.trim() {
            "input" => match v.trim() {
                "any" => manifest.input = ProgramInput::Any,
                "text" => manifest.input = ProgramInput::Text,
                "typed" => manifest.input = ProgramInput::Typed,
                _ => {}
            },
            "output" => match v.trim() {
                "direct" => manifest.output = ProgramOutput::Direct,
                "text" => manifest.output = ProgramOutput::Text,
                "typed" => manifest.output = ProgramOutput::Typed,
                _ => {}
            },
            "grant" => match v.trim() {
                "CAP_APP_TERMCTRL" => {
                    manifest.grants.push((CAP_APP_TERMCTRL, AccessRights::READ | AccessRights::WRITE));
                }
                _ => {}
            },
            _ => {}
        }
    }

    Some(manifest)
}

fn load_manifest(name: &str) -> Option<ProgramManifest> {
    let manifest_name = format!("{}.mf", name);
    let manifest_dir = PathBuf::from_str("/System/Manifests/");
    let manifest_path = manifest_dir.join(&Path::new(manifest_name.as_str()));

    parse_manifest(&manifest_path.as_path())
}

pub fn launch_command(name: &str, args: &[String], context: &ShellContext) -> ShellResult {
    let manifest = load_manifest(name).unwrap_or_default();

    match manifest.output {
        ProgramOutput::Direct | ProgramOutput::Text => {
            let spawned = match spawn_command(name, args, context, env::source(), CommandSink::Terminal) {
                Ok(s) => s,
                Err(r) => return r,
            };
            wait_process(name, spawned.process)
        }
        ProgramOutput::Typed => {
            let spawned = match spawn_command(name, args, context, env::source(), CommandSink::Pipe) {
                Ok(s) => s,
                Err(r) => return r,
            };

            let SpawnedOutput::Piped { socket, .. } = spawned.output else {
                return ShellResult::FailedToLaunch(
                    name.into(),
                    Error::invalid_argument("typed command did not produce piped output".into()),
                );
            };

            let render_result = render_typed_stream(socket, HandleWriter::new(env::sink()));
            let wait_result = spawned.process.wait();
            let _ = unset_raw_mode();

            match (render_result, wait_result) {
                (Ok(()), Ok(exit)) => ShellResult::Launched(exit),
                (_, Err(e)) => ShellResult::FailedToLaunch(name.into(), e),
                (Err(e), _) => ShellResult::FailedToRender(name.into(), e),
            }
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

pub fn build_exec(name: &str, args: &[String], context: &ShellContext) -> Result<Exec, Error> {
    let child_fs_rights = AccessRights::READ |
        AccessRights::WRITE |
        AccessRights::CREATE |
        AccessRights::EXECUTE |
        AccessRights::TRAVERSE |
        AccessRights::REMOVE |
        AccessRights::LIST;

    let exec = Exec::new(name.into())
        .source(env::source())
        .cwd(context.cwd_handle(), AccessRights::TRAVERSE)
        .args(args)
        .root_rights(child_fs_rights);

    Ok(exec)
}

pub fn spawn_command(
    name: &str, args: &[String], context: &ShellContext, source: HandleID, sink: CommandSink,
) -> Result<SpawnedCommand, ShellResult> {
    let manifest = load_manifest(name).unwrap_or_default();

    if manifest.output == ProgramOutput::Direct && !matches!(sink, CommandSink::Terminal) {
        return Err(ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("direct program cannot be piped".into())));
    }

    let mut exec = build_exec(name, args, context)
        .map_err(|e| {
            if e.kind == ErrorKind::NotFound { ShellResult::NotFound(name.into()) } else { ShellResult::FailedToLaunch(name.into(), e) }
        })?
        .source(source);

    for grant in manifest.grants {
        exec = exec.grant(grant.0, grant.1).map_err(|e| ShellResult::FailedToLaunch(name.into(), e))?;
    }

    match sink {
        CommandSink::Terminal => {
            let process = exec.sink(env::sink()).spawn().map_err(|e| ShellResult::FailedToLaunch(name.into(), e))?;

            Ok(SpawnedCommand { process, output: SpawnedOutput::Terminal })
        }
        CommandSink::Pipe => {
            let (process, socket) = exec.spawn_piped_sink().map_err(|e| ShellResult::FailedToLaunch(name.into(), e))?;

            Ok(SpawnedCommand { process, output: SpawnedOutput::Piped { kind: manifest.output, socket } })
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
        SpawnedOutput::Piped { kind: ProgramOutput::Typed, socket } => render_typed_stream(socket, HandleWriter::new(env::sink())),
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

            check_pipe_compat(left_kind, right)?;

            let right_input = first_input_mode(right);

            let mut adapter_sockets = left_run.adapter_sockets;
            let mut adapter_threads = left_run.adapter_threads;

            let right_source = if left_kind == ProgramOutput::Typed && right_input == Some(ProgramInput::Text) {
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

    let manifest = load_manifest(exec.as_str()).unwrap_or_default();

    let actual_sink = match sink {
        CommandSink::Pipe => CommandSink::Pipe,
        CommandSink::Terminal if manifest.output == ProgramOutput::Typed => CommandSink::Pipe,
        CommandSink::Terminal => CommandSink::Terminal,
    };

    let spawned = spawn_command(exec.as_str(), args, context, source, actual_sink)?;

    Ok(PipelineRun {
        processes: vec![(exec.clone(), spawned.process)],
        output: spawned.output,
        adapter_sockets: Vec::new(),
        adapter_threads: Vec::new(),
    })
}

fn check_pipe_compat(left_output: ProgramOutput, right: &BaseNode) -> Result<(), ShellResult> {
    let Some(input) = first_input_mode(right) else {
        return Ok(());
    };

    let accepted = match input {
        ProgramInput::Any => true,
        ProgramInput::Typed => left_output == ProgramOutput::Typed,
        ProgramInput::Text => left_output == ProgramOutput::Text || left_output == ProgramOutput::Typed,
    };

    if accepted {
        Ok(())
    } else {
        Err(ShellResult::FailedToLaunch("pipeline".into(), Error::invalid_argument("pipeline type mismatch".into())))
    }
}

fn first_input_mode(node: &BaseNode) -> Option<ProgramInput> {
    match node {
        BaseNode::Cmd(CommandNode::Run { exec, .. }) => Some(load_manifest(exec.as_str()).unwrap_or_default().input),
        BaseNode::Pipe(left, _) => first_input_mode(left),
        _ => None,
    }
}
