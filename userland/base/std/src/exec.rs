use alloc::format;
use core::slice;

use vespertine_abi::protocol::{
    PacketHeader,
    PacketType,
    VESPER_MAGIC,
};
use vespertine_abi::tag::CAP_PROCMAN;
use vespertine_abi::{
    AccessRights, CapabilityGrant, CapabilityID, HandleID, Invocation, ProcInfo, ProcManOp, ProcOp, ProcSignal, Signal, SpawnCredentials, UserID, WaitOp,
};
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use vrt::once::OnceCell;
use vrt::syscall::{
    sys_close, sys_invoke, sys_send_signal,
};

use crate::broker::Broker;
use crate::fs::{
    Path,
    resolve,
};
use crate::socket::Socket;
use crate::{
    Error,
    ErrorKind,
    Read,
    env,
};

struct ProcManager {
    handle: HandleID,
}

impl ProcManager {
    pub fn request() -> Result<Self, Error> {
        let broker_handle = resolve(&Path::new("/System/Services/ProcManager"), AccessRights::READ).map_err(Error::from)?;
        let broker = Broker::from_handle(broker_handle);
        let handle = broker.request(CAP_PROCMAN, AccessRights::LIST | AccessRights::READ | AccessRights::CREATE | AccessRights::EXECUTE)?;
        Ok(Self { handle })
    }

    pub fn spawn(&self, op: ProcManOp) -> Result<HandleID, Error> {
        let result = sys_invoke(self.handle, &Invocation::ProcessManager(op)).map_err(Error::from)?;
        Ok(HandleID(result))
    }

    pub fn list(&self, offset: usize) -> Result<ReadProcesses, Error> {
        let (read_end, write_end) = Socket::new_pair()?;

        let op = ProcManOp::List { offset, sink: write_end.handle() };
        sys_invoke(self.handle, &Invocation::ProcessManager(op)).map_err(Error::from)?;

        write_end.close();
        Ok(ReadProcesses { read_end, finished: false })
    }

    pub fn open(&self, pid: usize, rights: AccessRights) -> Result<Process, Error> {
        let handle = sys_invoke(self.handle, &Invocation::ProcessManager(ProcManOp::Open { pid, rights })).map_err(Error::from)?;

        Ok(Process::from_handle(HandleID(handle)))
    }
}

impl Drop for ProcManager {
    fn drop(&mut self) { let _ = sys_close(self.handle); }
}

static PROC_MANAGER: OnceCell<ProcManager> = OnceCell::new();

fn process_manager() -> Result<&'static ProcManager, Error> { PROC_MANAGER.get_or_try_init(ProcManager::request) }

#[allow(dead_code)]
pub struct Process {
    handle: HandleID,
}

impl Process {
    pub fn from_handle(handle: HandleID) -> Self { Self { handle } }

    pub fn handle(&self) -> HandleID { self.handle }

    pub fn terminate(&self) -> Result<(), Error> {
        sys_invoke(self.handle, &Invocation::Proc(ProcOp::Terminate { reason: 0 })).map(|_| ()).map_err(Error::from)
    }

    pub fn info(&self) -> Result<ProcInfo, Error> {
        Self::info_from_handle(self.handle)
    }

    pub fn info_from_handle(handle: HandleID) -> Result<ProcInfo, Error> {
        let mut info = ProcInfo::zeroed();
        let op = Invocation::Proc(ProcOp::GetInfo {
            info_ptr: &mut info as *mut _ as usize,
        });
        sys_invoke(handle, &op).map_err(Error::from)?;
        Ok(info)
    }

    pub fn wait(&self) -> Result<ProcInfo, Error> {
        sys_invoke(self.handle, &Invocation::Wait(WaitOp::One(Signal::TERMINATED)))
            .map_err(|error| Error::from(error).context("waiting for termination"))?;
        self.info().map_err(|error| error.context("reading exit status"))
    }

    pub fn resume(&self) -> Result<(), Error> {
        sys_invoke(self.handle, &Invocation::Proc(ProcOp::Resume)).map(|_| ()).map_err(Error::from)
    }

    pub fn send_signal(&self, signal: ProcSignal) -> Result<(), Error> {
        sys_send_signal(self.handle, signal).map_err(Error::from)
    }

    pub fn send_kill(&self) -> Result<(), Error> {
        self.send_signal(ProcSignal::Kill)
    }

    pub fn send_interrupt(&self) -> Result<(), Error> {
        self.send_signal(ProcSignal::Interrupt)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NamespaceRights(AccessRights);

impl NamespaceRights {
    pub const NONE: Self = Self(AccessRights::new());
    pub const FULL: Self = Self(AccessRights::all());

    pub const fn from_access_rights(rights: AccessRights) -> Self {
        Self(rights)
    }

    pub const fn as_access_rights(self) -> AccessRights {
        self.0
    }
}

pub struct Exec {
    exec_handle: HandleID,
    owns_exec_handle: bool,
    name: String,
    argv0: String,
    args: Vec<String>,
    root: HandleID,
    cwd: HandleID,
    source: HandleID,
    sink: HandleID,
    credentials: SpawnCredentials,
    capabilities: Vec<CapabilityGrant>,
    namespace_rights: NamespaceRights,
    cwd_rights: AccessRights,
    start_suspended: bool,
}

// --------------------------------------------------------//
// Handle table convention:
// Handle(0) = process root namespace
// Handle(1) = self handle
// Handle(2) = source
// Handle(3) = sink
// Handle(4) = memory pool
// Handle(5) = cwd
// --------------------------------------------------------//

impl Exec {
    pub fn from_handle(exec_handle: HandleID, argv0: String) -> Self { Self::build(exec_handle, false, argv0) }

    pub fn open(path: &Path<'_>, argv0: String) -> Result<Self, Error> {
        let exec_handle = resolve(path, AccessRights::READ | AccessRights::EXECUTE)
            .map_err(|error| error.context(format_args!("cannot open executable '{}'", path.as_str())))?;
        Ok(Self::build(exec_handle, true, argv0))
    }

    pub fn open_canonical(name: &str) -> Result<Self, Error> {
        let path_str = format!("/Programs/{}.app/bin/{}", name, name);
        let path = Path::new(&path_str);
        Self::open(&path, name.into())
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = name.into();
        self
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

    pub fn cwd(mut self, handle: HandleID, rights: AccessRights) -> Self {
        self.cwd = handle;
        self.cwd_rights = rights;
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

    pub fn user(mut self, user: UserID) -> Self {
        self.credentials = SpawnCredentials::User { user };
        self
    }

    pub fn grant(self, capability: CapabilityID, rights: AccessRights) -> Result<Self, Error> {
        let grant =
            env::capability(capability).ok_or(Error { kind: ErrorKind::NotFound, message: "Must own capability to grant it".into() })?;
        self.grant_new(grant.id, capability, rights)
    }

    pub fn grant_new(mut self, id: HandleID, capability: CapabilityID, rights: AccessRights) -> Result<Self, Error> {
        let grant = CapabilityGrant { id, capability, rights };
        self.capabilities.push(grant);
        Ok(self)
    }

    pub fn inherit_capabilities(mut self) -> Self {
        self.capabilities.extend(env::capabilities());
        self
    }

    pub fn inherit_credentials(mut self) -> Self {
        self.credentials = SpawnCredentials::Inherit;
        self
    }

    pub fn namespace_rights(mut self, rights: NamespaceRights) -> Self {
        self.namespace_rights = rights;
        self
    }

    pub fn start_suspended(mut self) -> Self {
        self.start_suspended = true;
        self
    }

    pub fn spawn(self) -> Result<Process, Error> {
        // null terminated args buffer
        let mut args_buf = Vec::new();
        args_buf.extend_from_slice(self.argv0.as_bytes()); // append program name as arg[0]
        args_buf.push(0);
        for arg in &self.args {
            args_buf.extend_from_slice(arg.as_bytes());
            args_buf.push(0);
        }
        let op = ProcManOp::Spawn {
            name_ptr: self.name.as_ptr() as usize,
            name_len: self.name.len(),
            exec_handle: self.exec_handle,
            root_handle: self.root,
            root_rights: self.namespace_rights.as_access_rights(),
            source: self.source,
            sink: self.sink,
            credentials: self.credentials,
            cwd_handle: self.cwd,
            cwd_rights: self.cwd_rights,
            capabilities_ptr: self.capabilities.as_ptr() as usize,
            capabilities_len: self.capabilities.len(),
            args_buffer_ptr: args_buf.as_ptr() as usize,
            args_buffer_len: args_buf.len(),
            start_suspended: self.start_suspended,
        };

        let manager = process_manager()
            .map_err(|error| error.context("cannot acquire the process manager"))?;
        let handle = manager.spawn(op)
            .map_err(|error| error.context(format_args!("cannot spawn '{}'", self.name)))?;
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

    pub fn build(exec_handle: HandleID, owns_exec_handle: bool, argv0: String) -> Self {
        Self {
            exec_handle,
            owns_exec_handle,
            name: argv0.clone(),
            argv0,
            args: Vec::new(),
            root: env::root(),
            cwd: env::cwd(),
            source: env::source(),
            sink: env::sink(),
            credentials: SpawnCredentials::Inherit,
            capabilities: Vec::new(),
            namespace_rights: NamespaceRights::NONE,
            cwd_rights: AccessRights::TRAVERSE | AccessRights::LIST,
            start_suspended: false,
        }
    }
}

impl Drop for Exec {
    fn drop(&mut self) {
        if self.owns_exec_handle {
            let _ = sys_close(self.exec_handle);
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) { let _ = sys_close(self.handle); }
}

pub struct ReadProcesses {
    read_end: Socket,
    finished: bool,
}

impl ReadProcesses {
    fn read_entry(&self) -> Result<Option<ProcInfo>, Error> {
        let mut header = PacketHeader::default();
        let header_bytes = unsafe {
            slice::from_raw_parts_mut((&mut header as *mut PacketHeader).cast::<u8>(), size_of::<PacketHeader>())
        };
        self.read_end.read_exact(header_bytes)
            .map_err(|error| error.context("reading process list header or end marker"))?;

        if header.magic != VESPER_MAGIC || header.version != 1 || header.packet_type != PacketType::ProcessInfo as u32 {
            return Err(Error::invalid_encoding("invalid process list header".into()));
        }

        if header.payload_len == 0 {
            return Ok(None);
        }

        if header.payload_len as usize != size_of::<ProcInfo>() {
            return Err(Error::invalid_encoding("invalid process entry size".into()));
        }

        let mut info = ProcInfo::zeroed();
        let info_bytes = unsafe {
            slice::from_raw_parts_mut((&mut info as *mut ProcInfo).cast::<u8>(), size_of::<ProcInfo>())
        };
        self.read_end.read_exact(info_bytes)
            .map_err(|error| error.context("reading process entry payload"))?;

        Ok(Some(info))
    }
}

impl Iterator for ReadProcesses {
    type Item = Result<ProcInfo, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        let result = self.read_entry();
        if !matches!(&result, Ok(Some(_))) {
            self.finished = true;
        }
        result.transpose()
    }
}

pub fn list_processes() -> Result<ReadProcesses, Error> { process_manager()?.list(0) }

pub fn open_process(pid: usize, rights: AccessRights) -> Result<Process, Error> { process_manager()?.open(pid, rights) }
