use vespertine_abi::{
    AccessRights, CapabilityGrant, CapabilityID, HandleID, Invocation, ProcManOp, ProcOp, ProcStatus, ProcessExitInfo, ProcessExitKind, Signal, WaitOp, tag::CAP_PROCMAN
};
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use vespertine_rt::{once::OnceCell, syscall::{sys_close, sys_invoke, sys_yield}};

use crate::{
    Error, ErrorKind, broker::Broker, env, fs::{Dir, walk_path}, socket::Socket
};

struct ProcManager {
    handle: HandleID,
}

impl ProcManager {
    pub fn request() -> Result<Self, Error> {
        let broker_handle = walk_path("/System/Services/ProcManager", env::root()).map_err(Error::from)?;
        let broker = Broker::from_handle(broker_handle);
        let handle = broker.request(CAP_PROCMAN, AccessRights::CREATE | AccessRights::EXECUTE)?;
        Ok(Self { handle })
    }

    pub fn spawn(&self, op: ProcManOp) -> Result<HandleID, Error> {
        let result = sys_invoke(self.handle, &Invocation::ProcessManager(op)).map_err(Error::from)?;
        Ok(HandleID(result))
    }
}

impl Drop for ProcManager {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}

static PROC_MANAGER: OnceCell<ProcManager> = OnceCell::new();

fn process_manager() -> Result<&'static ProcManager, Error> {
    PROC_MANAGER.get_or_try_init(ProcManager::request)
}

#[allow(dead_code)]
pub struct Process {
    handle: HandleID,
}

impl Process {
    pub fn wait(&self) -> Result<ProcessExitInfo, Error> {
        sys_invoke(
            self.handle,
            &Invocation::Wait(WaitOp::One(Signal::TERMINATED)),
        )
        .map_err(Error::from)?;
    
        let mut info = ProcessExitInfo::running();
    
        sys_invoke(
            self.handle,
            &Invocation::Proc(ProcOp::GetExitInfo {
                info_ptr: &mut info as *mut _ as usize,
            }),
        )
        .map_err(Error::from)?;
    
        Ok(info)
    }
}

pub struct Exec {
    exec_name: String,
    args: Vec<String>,
    root: HandleID,
    source: HandleID,
    sink: HandleID,
    capabilities: Vec<CapabilityGrant>,
    root_rights: AccessRights,
}

// --------------------------------------------------------//
// Handle table convention:
// Handle(0) = process root namespace
// Handle(1) = self handle
// Handle(2) = source
// Handle(3) = sink
// Handle(4) = memory pool
// --------------------------------------------------------//

impl Exec {
    pub fn new(name: String) -> Self {
        // common case with child inheriting root/source/sink from parent
        // but no extra handles and no rights
        Self {
            exec_name: name,
            args: Vec::new(),
            root: env::root(),
            source: env::source(),
            sink: env::sink(),
            capabilities: Vec::new(),
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

    pub fn grant(self, capability: CapabilityID, rights: AccessRights) -> Result<Self, Error> {
        let grant = env::capability(capability).ok_or(Error {
            kind: ErrorKind::NotFound,
            message: "Must own capability to grant it".into(),
        })?;
        self.grant_new(grant.id, capability, rights)
    }

    pub fn grant_new(
        mut self,
        id: HandleID,
        capability: CapabilityID,
        rights: AccessRights,
    ) -> Result<Self, Error> {
        let grant = CapabilityGrant { id, capability, rights };
        self.capabilities.push(grant);
        Ok(self)
    }

    pub fn inherit_capabilities(mut self) -> Self {
        self.capabilities.extend(env::capabilities());
        self
    }

    pub fn root_rights(mut self, rights: AccessRights) -> Self {
        self.root_rights = rights;
        self
    }

    pub fn spawn(self) -> Result<Process, Error> {
        let exec = Dir::from(env::root())
            .subdir("Programs")?
            .lookup(self.exec_name.as_str())?;

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
            capabilities_ptr: self.capabilities.as_ptr() as usize,
            capabilities_len: self.capabilities.len(),
            args_buffer_ptr: args_buf.as_ptr() as usize,
            args_buffer_len: args_buf.len(),
        };

        let handle = process_manager()?.spawn(op)?;
        Ok(Process { handle })
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
