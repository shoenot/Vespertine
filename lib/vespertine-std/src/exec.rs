use core::slice;

use alloc::format;

use vespertine_abi::protocol::{PacketFlags, PacketHeader, PacketType, VESPER_MAGIC};
use vespertine_abi::tag::CAP_PROCMAN;
use vespertine_abi::{
    AccessRights, CapabilityGrant, CapabilityID, HandleID, Invocation, ProcInfo, ProcManOp, ProcOp, ProcState, ProcTermReason, Signal, SpawnCredentials, UserID, WaitOp
};
extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use vespertine_rt::once::OnceCell;
use vespertine_rt::syscall::{
    sys_close,
    sys_invoke,
};

use crate::broker::Broker;
use crate::fs::{
    Path,
    resolve,
};
use crate::socket::Socket;
use crate::{
    Error, ErrorKind, Read, env
};

struct ProcManager {
    handle: HandleID,
}

impl ProcManager {
    pub fn request() -> Result<Self, Error> {
        let broker_handle = resolve(&Path::new("/System/Services/ProcManager"), AccessRights::READ).map_err(Error::from)?;
        let broker = Broker::from_handle(broker_handle);
        let handle = broker.request(CAP_PROCMAN, 
            AccessRights::LIST | AccessRights::READ | AccessRights::CREATE | AccessRights::EXECUTE)?;
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
        let handle = sys_invoke(
            self.handle,
            &Invocation::ProcessManager(ProcManOp::Open { pid, rights }),
        ).map_err(Error::from)?;
    
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
        sys_invoke(self.handle, &Invocation::Proc(ProcOp::Terminate { reason: 0 }))
            .map(|_| ())
            .map_err(Error::from)
    }

    pub fn info(&self) -> Result<ProcInfo, Error> {
        let mut info = ProcInfo::zeroed();
        sys_invoke(self.handle, &Invocation::Proc(ProcOp::GetInfo { info_ptr: &mut info as *mut _ as usize }))
            .map_err(Error::from)?;
        Ok(info)
    }

    pub fn wait(&self) -> Result<ProcInfo, Error> {
        sys_invoke(self.handle, &Invocation::Wait(WaitOp::One(Signal::TERMINATED))).map_err(Error::from)?;
        self.info()
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
    root_rights: AccessRights,
    cwd_rights: AccessRights,
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
        let exec_handle = resolve(path, AccessRights::READ | AccessRights::EXECUTE)?;
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

    pub fn root_rights(mut self, rights: AccessRights) -> Self {
        self.root_rights = rights;
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
            root_rights: self.root_rights,
            source: self.source,
            sink: self.sink,
            credentials: self.credentials,
            cwd_handle: self.cwd,
            cwd_rights: self.cwd_rights,
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
            root_rights: AccessRights::new(),
            cwd_rights: AccessRights::TRAVERSE | AccessRights::LIST,
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

impl Iterator for ReadProcesses {
    type Item = ProcInfo;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished { return None; }

        let mut header = PacketHeader::default();
        let header_bytes = unsafe {
            slice::from_raw_parts_mut(&mut header as *mut PacketHeader as *mut u8,
            size_of::<PacketHeader>())
        };

        if self.read_end.read_exact(header_bytes).is_err() {
            self.finished = true;
            return None;
        }

        if header.magic != VESPER_MAGIC ||
            header.version != 1 ||
            header.packet_type != PacketType::ProcessInfo as u32 ||
            header.payload_len as usize != size_of::<ProcInfo>()
        {
            self.finished = true;
            return None;
        }

        let mut info = ProcInfo::zeroed();

        let info_bytes = unsafe {
            slice::from_raw_parts_mut(&mut info as *mut ProcInfo as *mut u8, size_of::<ProcInfo>())
        };

        if self.read_end.read_exact(info_bytes).is_err() {
            self.finished = true;
            return None;
        }

        if !header.packet_flags.contains(PacketFlags::HAS_NEXT) { self.finished = true; }

        Some(info)
    }
}

pub fn list_processes() -> Result<ReadProcesses, Error> {
    process_manager()?.list(0)
}

pub fn open_process(pid: usize, rights: AccessRights) -> Result<Process, Error> {
    process_manager()?.open(pid, rights)
}
