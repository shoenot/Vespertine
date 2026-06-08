use vespertine_abi::{
    AccessRights, HandleGrant, HandleID, Invocation, ProcManOp, ProcStatus, tag::TAG_SYS_PROCMAN
};
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use vespertine_rt::syscall::{sys_close, sys_invoke};

use crate::{Error, ErrorKind::{self, NotFound}, env, fs::Dir, socket::Socket};

#[allow(dead_code)]
pub struct Process {
    handle: HandleID,
}

impl Process {
    pub fn wait(&self) -> Result<(), Error> {
        loop {
            let mut status = ProcStatus {
                pid: 0,
                active_threads: 0,
                is_terminated: false,
                memory_usage: 0,
            };
            let op = vespertine_abi::ProcOp::GetStatus {
                status_ptr: &mut status as *mut _ as usize,
            };
            let res = sys_invoke(self.handle, &Invocation::Proc(op));
            if res.is_err() || status.is_terminated {
                break;
            }
            // yield or sleep
            crate::clock::Clock::sleep_ms(10);
        }
        Ok(())
    }
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

// --------------------------------------------------------//
// Handle table convention: 
// Handle(0) = process root namespace
// Handle(1) = self handle
// Handle(2) = source
// Handle(3) = sink
// --------------------------------------------------------//

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

    pub fn grant(mut self, tag: usize, rights: AccessRights) -> Result<Self, Error> {
        let handle = env::find_tag(tag)
            .ok_or(Error { kind: ErrorKind::NotFound, message: "Must own handle to grant it".into() })?.id;
        self.grant_new(handle, tag, rights)
    }

    pub fn grant_new(mut self, id: HandleID, tag: usize, rights: AccessRights) -> Result<Self, Error> {
        let grant = HandleGrant { id, tag, rights };
        self.extra_handles.push(grant);
        Ok(self)
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
        let pm = env::find_tag(TAG_SYS_PROCMAN).ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "[ERROR] Process manager capability not found.".into(),
        })?;

        let exec = Dir::from(env::root())
            .subdir("Programs")?
            .lookup(self.exec_name)?;

        // null terminated args buffer
        let mut args_buf = Vec::new();
        args_buf.extend_from_slice(self.exec_name.as_bytes()); // append program name as arg[0]
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
            extra_handles_ptr: self.extra_handles.as_ptr() as usize,
            extra_handles_len: self.extra_handles.len(),
            args_buffer_ptr: args_buf.as_ptr() as usize,
            args_buffer_len: args_buf.len(),
        };

        let handle = sys_invoke(pm.id, &Invocation::ProcessManager(op)).map_err(Error::from)?;

        Ok(Process {
            handle: HandleID(handle),
        })
    }

    pub fn spawn_piped_source(self) -> Result<(Process, Socket), Error> {
        let (rx, tx) = Socket::new_pair()?;
        let proc = self.source(rx.handle()).spawn()?;
        drop(rx);
        Ok((proc, tx))
    }

    pub fn spawn_piped_sink(self) -> Result<(Process, Socket), Error> {
        let (rx, tx) = Socket::new_pair()?;
        let proc = self.sink(tx.handle()).spawn()?;
        drop(tx);
        Ok((proc, rx))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}
