use vespertine_abi::app::hesper::AppIoMode;
use vrt::mutex::Mutex;
use vrt::syscall::sys_yield;
use vrt::thread as rt_thread;
use vstd::prelude::*;
use vstd::term::unset_raw_mode;
use vstd::typed::render_typed_stream;
use vstd::{
    HandleWriter,
    Process,
};

use crate::parser::ast::{
    BaseNode,
    CommandNode,
};
use crate::runtime::env::ShellContext;
use crate::sys::ShellResult;
use crate::sys::launch::{
    SpawnedOutput,
    spawn_command,
};
use crate::sys::metadata::load_program_metadata;
use crate::sys::mode::{
    CommandSink,
    choose_mode,
};

struct PipelineRun {
    processes: Vec<(String, Process)>,
    output: SpawnedOutput,
    adapter_threads: Vec<Adapter>,
}

struct Adapter {
    producer: String,
    thread: rt_thread::JoinHandle,
    result: Arc<Mutex<Option<Result<(), Error>>>>,
}

fn start_adapter(producer: String, source: Socket, sink: Socket) -> Result<Adapter, ShellResult> {
    let result = Arc::new(Mutex::new(None));
    let worker_result = Arc::clone(&result);

    let thread = rt_thread::spawn(move || {
        let outcome = render_typed_stream(source, HandleWriter::new(sink.handle()));
        // close the write end so the consumer can see EOF
        drop(sink);
        *worker_result.lock() = Some(outcome);
    })
    .map_err(|e| ShellResult::FailedToLaunch(
            producer.clone(), 
            Error::from(e).context("cannot start typed-to-text adapter"),
    ))?;

    Ok(Adapter { producer, thread, result })
}

pub fn launch_base(base: BaseNode, context: &ShellContext) -> ShellResult {
    let run = match spawn_base(&base, context, env::source(), CommandSink::Terminal) {
        Ok(run) => run,
        Err(result) => return result,
    };

    if let Err(result) = resume_processes(&run.processes) {
        return result;
    }

    let render_result = match run.output {
        SpawnedOutput::Terminal => Ok(()),
        SpawnedOutput::Piped { kind: AppIoMode::Typed, socket } => render_typed_stream(socket, HandleWriter::new(env::sink())),
        SpawnedOutput::Piped { .. } => Err(Error::invalid_argument("final pipeline node must be terminal-renderable".into())),
    };

    let mut final_result = ShellResult::None;
    for (name, process) in run.processes {
        match process.wait() {
            Ok(exit) => final_result = ShellResult::Launched(exit),
            Err(error) => return ShellResult::FailedToWait(name, error),
        }
    }
    let mut adapter_failure = None;
    for adapter in run.adapter_threads {
        while !adapter.thread.is_done() {
            sys_yield();
        }

        let outcome = adapter.result.lock().take().unwrap_or_else(|| {
            Err(Error::unknown("pipeline adapter completed without a result".into()))
        });

        if let Err(e) = outcome {
            if adapter_failure.is_none() {
                adapter_failure = Some(ShellResult::from_render_error(&adapter.producer, e));
            }
        }
    }

    let _ = unset_raw_mode();

    match render_result {
        Ok(()) => adapter_failure.unwrap_or(final_result),
        Err(e) => ShellResult::from_render_error("pipeline", e),
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

            let mut adapter_threads = left_run.adapter_threads;

            let right_run = if left_kind == AppIoMode::Typed && right_input == Some(AppIoMode::Text) {
                let producer = left_run.processes.last()
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| "pipeline".into());

                let (text_rx, text_tx) = Socket::new_pair().map_err(|e| {
                    ShellResult::FailedToLaunch(producer.clone(), e.context("cannot created typed-to-text adapter pipe"))
                })?;

                let right_run = spawn_base(right, context, text_rx.handle(), sink)?;
                drop(text_rx);

                match start_adapter(producer, pipe_source, text_tx) {
                    Ok(adapter) => adapter_threads.push(adapter),
                    Err(e) => {
                        for (_, proc) in left_run.processes.iter().chain(&right_run.processes) {
                            let _ = proc.terminate();
                        }
                        return Err(e);
                    }
                }
                right_run
            } else {
                let right_run = spawn_base(right, context, pipe_source.handle(), sink)?;
                drop(pipe_source);
                right_run
            };

            let mut processes = left_run.processes;
            processes.extend(right_run.processes);
            adapter_threads.extend(right_run.adapter_threads);

            Ok(PipelineRun { processes, output: right_run.output, adapter_threads })
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

fn resume_processes(processes: &[(String, Process)]) -> Result<(), ShellResult> {
    for (name, process) in processes {
        process.resume().map_err(|error| ShellResult::FailedToLaunch(name.clone(), error))?;
    }
    Ok(())
}
