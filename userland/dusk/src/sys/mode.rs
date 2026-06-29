use vabi::app::hesper::AppIoMode;
use vstd::prelude::*;

use crate::sys::ShellResult;
use crate::sys::metadata::ProgramMetadata;

#[derive(Debug, Clone, Copy)]
pub(super) enum CommandSink {
    Terminal,
    Pipe,
}

pub(super) fn choose_mode(name: &str, args: &[String], metadata: ProgramMetadata, sink: CommandSink) -> Result<AppIoMode, ShellResult> {
    match sink {
        CommandSink::Terminal => choose_terminal_mode(name, args, metadata),
        CommandSink::Pipe => choose_pipe_mode(name, metadata),
    }
}

pub(super) fn needs_empty_direct_source(input: AppIoMode, mode: AppIoMode) -> bool {
    input == AppIoMode::Typed && mode != AppIoMode::Terminal
}

fn choose_terminal_mode(name: &str, args: &[String], metadata: ProgramMetadata) -> Result<AppIoMode, ShellResult> {
    if name == "sys" && args.first().map(|arg| arg.as_str()) == Some("top") && metadata.modes.contains_mode(AppIoMode::Terminal) {
        return Ok(AppIoMode::Terminal);
    }

    if metadata.modes.contains_mode(metadata.default_mode) {
        return Ok(metadata.default_mode);
    }

    Err(ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("application default mode is not supported".into())))
}

fn choose_pipe_mode(name: &str, metadata: ProgramMetadata) -> Result<AppIoMode, ShellResult> {
    if metadata.modes.contains_mode(AppIoMode::Typed) {
        Ok(AppIoMode::Typed)
    } else if metadata.modes.contains_mode(AppIoMode::Text) {
        Ok(AppIoMode::Text)
    } else {
        Err(ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("application cannot run in a pipeline".into())))
    }
}
