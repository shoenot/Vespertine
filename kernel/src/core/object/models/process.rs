use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::{format, vec};
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};
use core::task::{
    Context,
    Poll,
    Waker,
};

use async_trait::async_trait;
use hal::usercopy::{
    safe_copy_from,
    safe_copy_to,
};
use vespertine_abi::op::ProcOp;
use vespertine_abi::{
    AccessRights,
    HandleID,
    Invocation,
    ProcInfo,
    ProcState,
    ProcTermReason,
    Signal,
    WaitItem,
    WaitOp,
};
use vespertine_common::lock::TicketLock;

use crate::arch::get_core_data;
use crate::core::asynchronous::waiter::{
    AsyncWaiter,
    WaiterList,
    wake_all,
};
use crate::core::object::handle::HandleTable;
use crate::core::object::help::RightsWrapper;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::thread::Thread;
use crate::core::object::obj::{
    KernelObject,
    ObjectWaitFuture,
    matching_signals,
};
use crate::core::security::credentials::Credentials;
use crate::core::sync::RwLock;
use crate::core::thread::dispatch::{
    cancel_blocked_thread,
    reschedule_thread_core,
    spawn_user_thread,
    wake_thread,
};
use crate::core::thread::priority::ThreadPriority;
use crate::core::thread::wait::WaitQueue;
use crate::core::thread::{
    ThreadControlBlock,
    ThreadState,
    get_current_process,
};
use crate::memory::vmm::VirtMemManager;
use crate::memory::{
    ALLOCATOR,
    kernel_heap_allocated,
};
use crate::util::write_to_msr;

pub static GLOBAL_PID: AtomicUsize = AtomicUsize::new(0);
static PROCESS_REGISTRY: RwLock<BTreeMap<usize, Process>> = RwLock::new(BTreeMap::new());

pub fn register_process(process: &Process) { PROCESS_REGISTRY.write().insert(process.proc_id, process.clone()); }

pub fn unregister_process(pid: usize) { PROCESS_REGISTRY.write().remove(&pid); }

pub fn process_snapshot() -> Vec<Process> { PROCESS_REGISTRY.read().values().cloned().collect() }

pub fn find_process(pid: usize) -> Option<Process> { PROCESS_REGISTRY.read().get(&pid).cloned() }

pub fn get_new_pid() -> usize { GLOBAL_PID.fetch_add(1, core::sync::atomic::Ordering::Relaxed) }

pub type Process = Arc<ProcessControlBlock>;

#[repr(C)]
#[derive(Debug)]
pub struct ProcessControlBlock {
    pub proc_id: usize,
    pub proc_name: String,
    pub credentials: Credentials,

    pub handles: RwLock<HandleTable>,
    pub vmm: RwLock<VirtMemManager>,
    pub pml4_addr: usize,

    pub threads: RwLock<Vec<*mut ThreadControlBlock>>,
    pub active_threads: AtomicUsize,

    pub lifecycle: TicketLock<ProcLifecycle>,
    pub completion_waiters: TicketLock<WaiterList>,

    pub futexes: RwLock<BTreeMap<usize, WaitQueue>>,
}

unsafe impl Send for ProcessControlBlock {}
unsafe impl Sync for ProcessControlBlock {}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ProcLifecycle {
    Running,
    Terminating(ProcTermination),
    Terminated(ProcTermination),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ProcTermination {
    pub reason: ProcTermReason,
    pub code: u32,
    pub detail: usize,
}

impl ProcTermination {
    pub const fn exited(code: u32) -> Self { Self { reason: ProcTermReason::Exited, code, detail: 0 } }

    pub const fn terminated(reason: u32) -> Self { Self { reason: ProcTermReason::Terminated, code: reason, detail: 0 } }

    pub const fn faulted(code: u32, detail: usize) -> Self { Self { reason: ProcTermReason::Faulted, code, detail } }
}

struct WaitManyFuture<'a> {
    process: &'a ProcessControlBlock,
    items_ptr: usize,
    count: usize,
    waiter: Arc<AsyncWaiter>,
}

impl Drop for WaitManyFuture<'_> {
    fn drop(&mut self) { self.waiter.deactivate(); }
}

impl WaitManyFuture<'_> {
    fn load_items_and_endpoints(&self) -> Result<(Vec<WaitItem>, Vec<Arc<dyn KernelObject>>), InvocationError> {
        let mut items = vec![WaitItem { handle: HandleID(0), signal: Signal(0), pending: Signal(0) }; self.count];
        if !safe_copy_from(items.as_mut_ptr() as *mut u8, self.items_ptr as *const u8, self.count * size_of::<WaitItem>()) {
            return Err(InvocationError::InvalidPointer);
        }

        let mut objects = Vec::with_capacity(self.count);
        let table = self.process.handles.read();
        for item in &items {
            objects.push(table.resolve(item.handle, AccessRights::READ)?);
        }
        Ok((items, objects))
    }

    fn refresh_and_copy(&self, items: &mut [WaitItem], objects: &[Arc<dyn KernelObject>]) -> Result<bool, InvocationError> {
        let mut any_ready = false;
        for (item, object) in items.iter_mut().zip(objects) {
            item.pending = matching_signals(object.current_signals(), item.signal);
            any_ready |= item.pending != Signal(0);
        }
        if any_ready && !safe_copy_to(self.items_ptr as *mut u8, items.as_ptr() as *const u8, self.count * size_of::<WaitItem>()) {
            return Err(InvocationError::InvalidPointer);
        }
        Ok(any_ready)
    }
}

impl Future for WaitManyFuture<'_> {
    type Output = Result<usize, InvocationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let (mut items, objects) = match this.load_items_and_endpoints() {
            Ok(value) => value,
            Err(error) => return Poll::Ready(Err(error)),
        };

        match this.refresh_and_copy(&mut items, &objects) {
            Ok(true) => return Poll::Ready(Ok(0)),
            Ok(false) => {}
            Err(error) => return Poll::Ready(Err(error)),
        }

        for (item, object) in items.iter().zip(&objects) {
            if let Err(error) = object.register_waiter(item.signal, &this.waiter, cx.waker()) {
                return Poll::Ready(Err(error));
            }
        }

        match this.refresh_and_copy(&mut items, &objects) {
            Ok(true) => {
                this.waiter.deactivate();
                Poll::Ready(Ok(0))
            }
            Ok(false) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

impl ProcessControlBlock {
    pub fn new(init_table: HandleTable, name: String, credentials: Credentials) -> Process {
        let vmm = VirtMemManager::new(&ALLOCATOR);
        let pml4_addr = vmm.get_pml4_addr();
        let proc = Arc::new(Self {
            proc_id: get_new_pid(),
            proc_name: name,
            credentials,

            handles: RwLock::new(init_table),
            vmm: RwLock::new(vmm),
            pml4_addr,

            threads: RwLock::new(Vec::new()),
            active_threads: AtomicUsize::new(0),

            lifecycle: TicketLock::new(ProcLifecycle::Running),
            completion_waiters: TicketLock::new(WaiterList::new()),

            futexes: RwLock::new(BTreeMap::new()),
        });
        register_process(&proc);
        proc
    }

    pub fn snapshot_info(&self) -> ProcInfo {
        let lc = *self.lifecycle.lock();
        let (state, term_reason, term_code, term_detail) = match lc {
            ProcLifecycle::Running => (ProcState::Running, ProcTermReason::None, 0, 0),
            ProcLifecycle::Terminating(t) => (ProcState::Terminating, t.reason, t.code, t.detail),
            ProcLifecycle::Terminated(t) => (ProcState::Terminated, t.reason, t.code, t.detail),
        };

        let memory_usage = if self.proc_id == 0 { kernel_heap_allocated() } else { self.vmm.read().get_resident_size() };

        let mut info = ProcInfo::zeroed();
        
        info.pid = self.proc_id;
        info.user = self.credentials.user();
        info.set_name(&self.proc_name);
        info.state = state;
        info.active_threads = self.active_threads.load(Ordering::Acquire);
        info.memory_usage = memory_usage;
        
        info.term_reason = term_reason;
        info.term_code = term_code;
        info.term_detail = term_detail;
        
        info
    }

    pub fn info(&self, ptr: *mut ProcInfo) -> Result<usize, InvocationError> {
        let info = self.snapshot_info();

        if !safe_copy_to(ptr as *mut u8, &info as *const ProcInfo as *const u8, size_of::<ProcInfo>()) {
            return Err(InvocationError::InvalidPointer);
        }
        Ok(0)
    }

    pub fn request_terminate(&self, termination: ProcTermination) -> bool {
        {
            let mut lc = self.lifecycle.lock();
            match *lc {
                ProcLifecycle::Running => {
                    *lc = ProcLifecycle::Terminating(termination);
                }
                ProcLifecycle::Terminating(_) | ProcLifecycle::Terminated(_) => return false,
            }
        }
        let threads = self.thread_snapshot();
        for thread in threads {
            if thread.is_null() {
                continue;
            }
            unsafe {
                (*thread).request_cancel();
                match (*thread).state() {
                    ThreadState::Blocked => {
                        cancel_blocked_thread(thread);
                    }
                    ThreadState::Running | ThreadState::Ready => {
                        reschedule_thread_core(thread);
                    }
                    ThreadState::Terminated => {}
                }
            }
        }
        true
    }

    pub fn finish_thread_exit(&self, thread: *mut ThreadControlBlock, normal_exit_code: u32) -> bool {
        if !self.unregister_thread(thread) {
            return false;
        }

        let prev = self.active_threads.fetch_sub(1, Ordering::AcqRel);
        if prev != 1 {
            return true;
        }

        let mut lc = self.lifecycle.lock();

        let termination = match *lc {
            ProcLifecycle::Running => ProcTermination::exited(normal_exit_code),
            ProcLifecycle::Terminating(t) => t,
            ProcLifecycle::Terminated(_) => return false,
        };

        *lc = ProcLifecycle::Terminated(termination);
        drop(lc);

        self.handles.write().clear();
        let wakers = self.completion_waiters.lock().take_wakers();
        wake_all(wakers);
        true
    }

    pub fn is_terminating(&self) -> bool { matches!(*self.lifecycle.lock(), ProcLifecycle::Terminating(_) | ProcLifecycle::Terminated(_)) }

    pub fn is_terminated(&self) -> bool { matches!(*self.lifecycle.lock(), ProcLifecycle::Terminated(_)) }

    pub fn register_thread(&self, thread: *mut ThreadControlBlock) {
        if thread.is_null() {
            return;
        }
        let mut threads = self.threads.write();
        if !threads.iter().any(|&cd| cd == thread) {
            threads.push(thread);
        }
    }

    pub fn thread_snapshot(&self) -> Vec<*mut ThreadControlBlock> { self.threads.read().clone() }

    pub fn unregister_thread(&self, thread: *mut ThreadControlBlock) -> bool {
        let mut threads = self.threads.write();
        let old_len = threads.len();
        threads.retain(|&cd| cd != thread);
        threads.len() != old_len
    }
}

#[async_trait]
impl KernelObject for ProcessControlBlock {
    fn type_name(&self) -> &'static str { "Process" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Proc(ProcOp::Terminate { reason }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.request_terminate(ProcTermination::terminated(reason));
                Ok(0)
            }
            Invocation::Proc(ProcOp::GetInfo { info_ptr }) => {
                calling_rights.err_if_no(AccessRights::READ)?;
                self.info(info_ptr as *mut ProcInfo)
            }
            Invocation::Proc(ProcOp::Unmap { vaddr, len }) => {
                self.vmm.write().munmap(vaddr, len).map(|_| 0).map_err(|_| InvocationError::InvalidArgument)
            }
            Invocation::Proc(ProcOp::SpawnThread { entry, stack_top, arg, priority }) => {
                let tp = ThreadPriority::from(priority);
                let proc = get_current_process().ok_or(InvocationError::ThreadSpawnFail)?;
                let thread = spawn_user_thread(entry, stack_top, arg, tp, proc.clone());
                let obj = Arc::new(Thread { tcb: thread });
                let id = self.handles.write().insert(obj, AccessRights::all());
                Ok(id.0)
            }
            Invocation::Wait(WaitOp::One(signal)) => ObjectWaitFuture::new(self, signal).await,
            Invocation::Wait(WaitOp::Many { items_ptr, count }) => {
                if count == 0 || count > 64 {
                    return Err(InvocationError::InvalidArgument);
                }
                WaitManyFuture { process: self, items_ptr, count, waiter: AsyncWaiter::new() }.await
            }
            Invocation::Proc(ProcOp::SetFsBase { fs_base }) => {
                let current_thread = get_core_data().scheduler.get_current_thread();
                if current_thread.is_null() {
                    return Err(InvocationError::InvalidHandle);
                }
                unsafe {
                    (*current_thread).fs_base = fs_base;
                    write_to_msr(fs_base as u64, 0xC000_0100);
                }
                Ok(0)
            }
            Invocation::Proc(ProcOp::InsertHandle { source_handle, rights }) => {
                calling_rights.err_if_no(AccessRights::MUTATE)?;
                let caller = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let obj = caller.handles.read().resolve(source_handle, rights)?;
                let new_handle_id = self.handles.write().insert(obj, rights);
                Ok(new_handle_id.0)
            }
            Invocation::Proc(ProcOp::Mprotect { vaddr, len, prot }) => {
                self.vmm.write().mprotect(vaddr, len, prot).map(|_| 0).map_err(|_| InvocationError::InvalidArgument)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn current_signals(&self) -> Signal { if self.is_terminated() { Signal::TERMINATED } else { Signal(0) } }

    fn register_waiter(&self, requested: Signal, waiter: &Arc<AsyncWaiter>, waker: &Waker) -> Result<(), InvocationError> {
        if requested != Signal::TERMINATED {
            return Err(InvocationError::UnsupportedOperation);
        }

        self.completion_waiters.lock().register(waiter, waker);
        Ok(())
    }
}
