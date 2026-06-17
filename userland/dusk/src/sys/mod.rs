use core::fmt::Display;

use alloc::{fmt::format, format, string::{String, ToString}, vec::Vec};
use vespertine_abi::{AccessRights, CapabilityID, HandleID, ProcessExitInfo, tag::CAP_APP_TERMCTRL};
use vespertine_rt::println;
use vespertine_std::{Error, ErrorKind, Exec, HandleWriter, Process, Read, env, fs::{File, Path, PathBuf}, shell::render_typed_stream, socket::Socket, term::{check_raw_mode, clear_term_screen, unset_raw_mode}};

use crate::{error::ShellError, runtime::env::ShellContext};

#[derive(Debug, Clone)]
pub enum ShellResult {
    None,
    Launched(ProcessExitInfo),
    ChangeDirFail(String, Error),
    FailedToLaunch(String, Error),
    FailedToRender(String, Error),
    AccessDenied(String),
    NotFound(String),
}

impl Display for ShellResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellResult::None => core::fmt::Result::Ok(()),
            ShellResult::Launched(ei) => write!(f, "exit info: {:?}", ei),
            ShellResult::ChangeDirFail(path, err) => write!(f, "couldn't change dir to {}: {:?}", path, err),
            ShellResult::FailedToLaunch(name, err) => write!(f, "failed to launch {}: {:?}", name, err),
            ShellResult::FailedToRender(name, err) => write!(f, "failed to render {}: {:?}", name, err),
            ShellResult::AccessDenied(name) => write!(f, "access denied: {}", name),
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
    Piped {
        kind: ProgramOutput,
        socket: Socket,
    }
}

pub struct SpawnedCommand {
    pub process: Process,
    pub output: SpawnedOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramOutput {
    Direct,
    Typed,
    Text,
}

pub struct ProgramManifest {
    io: ProgramOutput,
    grants: Vec<(CapabilityID, AccessRights)>,
}

impl ProgramManifest {
    fn new() -> Self {
        Self { 
            io: ProgramOutput::Text,
            grants: Vec::new(),
        }
    }
}

impl Default for ProgramManifest {
    fn default() -> Self {
        Self::new()
    }
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
            "io" => match v.trim() {
                "direct" | "standard" => manifest.io = ProgramOutput::Direct,
                "text" => manifest.io = ProgramOutput::Text,
                "typed" => manifest.io = ProgramOutput::Typed,
                _ => {}
            },
            "grant" => match v.trim() {
                "CAP_APP_TERMCTRL" => {
                    manifest.grants.push((
                        CAP_APP_TERMCTRL,
                        AccessRights::READ | AccessRights::WRITE,
                    ));
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

    match manifest.io {
        ProgramOutput::Direct | ProgramOutput::Text => {
            let spawned = match spawn_command(name, args, context, env::source(), CommandSink::Terminal) {
                Ok(s) => s,
                Err(r) => return r,
            };
            wait_process(name, spawned.process)
        }
        ProgramOutput::Typed =>  {
            let spawned = match spawn_command(name, args, context, env::source(), CommandSink::Pipe) {
                Ok(s) => s,
                Err(r) => return r,
            };

            let SpawnedOutput::Piped { socket, .. } = spawned.output else {
                return ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("typed command did not produce piped output".into()));
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
    let child_fs_rights = 
        AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE | AccessRights::EXECUTE |
        AccessRights::TRAVERSE | AccessRights::REMOVE | AccessRights::LIST;

    let exec = Exec::new(name.into())
        .source(env::source())
        .cwd(context.cwd_handle(), AccessRights::TRAVERSE)
        .args(args)
        .root_rights(child_fs_rights);

    Ok(exec)
}

pub fn spawn_command(name: &str, args:&[String], context: &ShellContext, source: HandleID, sink: CommandSink) -> Result<SpawnedCommand, ShellResult> {
    let manifest = load_manifest(name).unwrap_or_default();

    if manifest.io == ProgramOutput::Direct && !matches!(sink, CommandSink::Terminal) {
        return Err(ShellResult::FailedToLaunch(name.into(), Error::invalid_argument("direct program cannot be piped".into())));
    }

    let mut exec = build_exec(name, args, context)
        .map_err(|e| {
            if e.kind == ErrorKind::NotFound {
                ShellResult::NotFound(name.into())
            } else {
                ShellResult::FailedToLaunch(name.into(), e)
            }
        })?.source(source);

    for grant in manifest.grants {
        exec = exec
            .grant(grant.0, grant.1)
            .map_err(|e| ShellResult::FailedToLaunch(name.into(), e))?;
    }

    match sink {
        CommandSink::Terminal => {
            let process = exec
                .sink(env::sink())
                .spawn()
                .map_err(|e| ShellResult::FailedToLaunch(name.into(), e))?;

            Ok(SpawnedCommand { process, output: SpawnedOutput::Terminal })
        },
        CommandSink::Pipe => {
            let (process, socket) = exec
                .spawn_piped_sink()
                .map_err(|e| ShellResult::FailedToLaunch(name.into(), e))?;

            Ok(SpawnedCommand { process, output: SpawnedOutput::Piped { kind: manifest.io, socket }})
        },
    }
}
