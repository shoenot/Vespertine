use vespertine_abi::{AccessRights, HandleGrant, HandleID, Invocation, ProcManOp, tag::TAG_SYS_PROCMAN};
extern crate alloc;
use alloc::vec::Vec;
use alloc::string::String;
use vespertine_rt::syscall::sys_invoke;

use crate::{Error, ErrorKind, env, fs::Dir};

pub struct Process {
    handle: HandleID,
}

pub struct Exec {
    exec_name: &'static str,
    args: Vec<String>,
    root: HandleID,
    source: HandleID,
    sink: HandleID,
    extra_handles: Vec<HandleGrant>,
    root_rights: AccessRights,
}

impl Exec {
    pub fn new(name: &'static str) -> Self {
        // common case with child inheriting root/source/sink from parent 
        // but no extra handles and no rights
        Self { 
            exec_name: name,
            args: Vec::new(),
            root: env::root(),
            source: env::source(),
            sink: env::sink(),
            extra_handles: Vec::new(),
            root_rights: AccessRights::new(),
        }
    }

    pub fn arg(mut self, arg: String) -> Self {
        self.args.push(arg);
        self
    }

    pub fn args(mut self, args: &[String]) -> Self {
        self.args.extend_from_slice(args);
        self
    }

    pub fn root(mut self, handle: HandleID) -> Self {
        self.root = handle;
        self
    }

    pub fn source(mut self, handle: HandleID) -> Self {
        self.source = handle;
        self
    }

    pub fn sink(mut self, handle: HandleID) -> Self {
        self.sink = handle;
        self
    }
    
    pub fn grant(mut self, grant: HandleGrant) -> Self {
        self.extra_handles.push(grant);
        self
    }

    pub fn inherit_capabilities(mut self) -> Self {
        self.extra_handles.extend(env::extra_handles());
        self
    }

    pub fn root_rights(mut self, rights: AccessRights) -> Self {
        self.root_rights = rights;
        self
    }

    pub fn spawn(self) -> Result<Process, Error> {
        let pm = env::find_tag(TAG_SYS_PROCMAN)
            .ok_or(Error { kind: ErrorKind::AccessDenied, message: "[ERROR] Process manager capability not found." })?;

        let exec = Dir::from(env::root())
            .subdir("Programs")?
            .lookup(self.exec_name)?;

        // null terminated args buffer
        let mut args_buf = Vec::new();
        args_buf.extend_from_slice(self.exec_name.as_bytes());    // append program name as arg[0]
        args_buf.push(0);
        for arg in &self.args {
            args_buf.extend_from_slice(arg.as_bytes());
            args_buf.push(0);
        }

        let op = ProcManOp::Spawn {
            exec_handle: exec,
            root_handle: self.root,
            root_rights: self.root_rights,
            source: self.source,
            sink: self.sink,
            extra_handles_ptr: self.extra_handles.as_ptr(),
            extra_handles_len: self.extra_handles.len(),
            args_buffer_ptr: args_buf.as_ptr(),
            args_buffer_len: args_buf.len(),
        };

        let handle = sys_invoke(pm.id, &Invocation::ProcessManager(op))
            .map_err(Error::from)?;

        Ok(Process { handle: HandleID(handle) })
    }

}
