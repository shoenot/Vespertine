use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use vespertine_abi::app::hesper::AppIoMode;
use vespertine_abi::HandleID;
use vespertine_rt::thread as rt_thread;
use vespertine_std::socket::Socket;
use vespertine_std::term::unset_raw_mode;
use vespertine_std::typed::render_typed_stream;
use vespertine_std::{
    env,
    Error,
    HandleWriter,
    Process,
};

use crate::parser::ast::{
    BaseNode,
    CommandNode,
};
use crate::runtime::env::ShellContext;
use crate::sys::launch::{
    spawn_command,
    SpawnedOutput,
};
use crate::sys::metadata::load_program_metadata;
use crate::sys::mode::{
    choose_mode,
    CommandSink,
};
use crate::sys::ShellResult;

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
        },
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
    let mode = choose_mode(exec.as_str(), args, metadata, sink)?;

    let actual_sink = match sink {
        CommandSink::Pipe => CommandSink::Pipe,
        CommandSink::Terminal if mode == AppIoMode::Typed => CommandSink::Pipe,
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
        AppIoMode::Terminal => false,
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
