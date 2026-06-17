use core::fmt::Display;

use alloc::{format, string::{String, ToString}, vec::Vec};
use vespertine_abi::{AccessRights, ProcessExitInfo, tag::CAP_APP_TERMCTRL};
use vespertine_rt::println;
use vespertine_std::{Error, ErrorKind, Exec, env, fs::{File, Path, PathBuf}, Read, term::{check_raw_mode, clear_term_screen, unset_raw_mode}};

use crate::{error::ShellError, runtime::env::ShellContext};

#[derive(Debug, Clone)]
pub enum ShellResult {
    None,
    Launched(ProcessExitInfo),
    ChangeDirFail(String, Error),
    FailedToLaunch(String, Error),
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
            ShellResult::AccessDenied(name) => write!(f, "access denied: {}", name),
            ShellResult::NotFound(name) => write!(f, "not found: {}", name),
        }
    }
}

pub fn launch_command(name: &str, args: &[String], context: &ShellContext) -> ShellResult {
    let res = build_exec(name, args, context).and_then(Exec::spawn);
    let info = match res {
        Ok(proc) => {
            match proc.wait() {
                Ok(exit) => ShellResult::Launched(exit),
                Err(error) => ShellResult::FailedToLaunch(name.to_string(), error),
            }
        },
        Err(error) if error.kind == ErrorKind::NotFound => {
            ShellResult::NotFound(name.to_string())
        },
        Err(error) => ShellResult::FailedToLaunch(name.to_string(), error),
    };

    let _ = unset_raw_mode();
    info
}

pub fn build_exec(name: &str, args: &[String], context: &ShellContext) -> Result<Exec, Error> {
    let child_fs_rights = 
        AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE | AccessRights::EXECUTE |
        AccessRights::TRAVERSE | AccessRights::REMOVE | AccessRights::LIST;

    let manifest_name = format!("{}.mf", name);
    let manifest_dir = PathBuf::from_str("/System/Manifests/");
    let manifest_path = manifest_dir.join(&Path::new(manifest_name.as_str()));

    let mut exec = Exec::new(name.into())
        .source(env::source())
        .cwd(context.cwd_handle(), AccessRights::TRAVERSE)
        .args(args)
        .root_rights(child_fs_rights);

    if let Some(manifest) = parse_manifest(&manifest_path.as_path()) {
        if manifest.io == ProgramOutput::Typed {
            exec = exec.sink(handle);
        } else {
            exec = exec.sink(env::sink());
        }
    }

    match name {
        "kilo" => exec.grant(CAP_APP_TERMCTRL, AccessRights::READ | AccessRights::WRITE),
        _ => Ok(exec),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramOutput {
    Direct,
    Typed,
    Text,
}

pub struct ProgramManifest {
    io: ProgramOutput,
}

impl ProgramManifest {
    fn new() -> Self {
        Self { io: ProgramOutput::Standard }
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
    let pairs: Vec<&str> = contents.split_whitespace().collect();

    let mut manifest = ProgramManifest::default();
    for pair in pairs {
        while let Some((k, v)) = pair.split_once(":") {
            match k.trim() {
                "io" => match v.trim() {
                    "standard" => manifest.io = ProgramOutput::Direct,
                    "text" => manifest.io = ProgramOutput::Text,
                    "typed" => manifest.io = ProgramOutput::Typed,
                    _ => {},
                },
                _ => {},
            }
        }
    }
    Some(manifest)
}
