use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::vec;
use hal::cpu::set_user_thread_pointer;
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
    AccessRights, HandleID, Invocation, ProcInfo, ProcSignal, ProcState, ProcTermReason, Signal, WaitItem, WaitOp,
};
use vespertine_common::lock::TicketLock;
use crate::object::mempool::MemPool;
use crate::process::notifications::{NotificationClaim, NotificationReceiver};
use crate::sync::TicketLock as IrqTicketLock;

use crate::executor::waiter::{
    AsyncWaiter,
    WaiterList,
    wake_all,
};
use crate::cpu::current_core_mut;
use crate::object::handle::HandleTable;
use crate::object::help::RightsWrapper;
use crate::object::invoke::{InvocationError, InvokeContext};
use crate::process::thread_object::ThreadObject;
use crate::object::obj::{
    KernelObject,
    ObjectWaitFuture,
    matching_signals,
};
use crate::security::credentials::Credentials;
use crate::sync::RwLock;
use crate::sched::dispatch::{
    create_user_tcb, enqueue_new_user_thread, request_thread_termination, reschedule_thread_core, spawn_user_thread, start_thread
};
use crate::sched::priority::ThreadPriority;
use crate::sched::wait::WaitQueue;
use crate::sched::{
    Thread, ThreadPin, ThreadState,
};
use crate::syscall::futex::FutexTable;
use super::current_process;
use crate::memory::vmm::{
    VirtMemManager, VmAccounting, VmError, VmPermissions,
};
use crate::memory::{
    ALLOCATOR,
    kernel_heap_allocated,
};

pub const DEFAULT_PROCESS_MEMORY_LIMIT: usize = 64 * 1024 * 1024;
pub const DEFAULT_PROCESS_MEMORY_MAXIMUM: usize = 1024 * 1024 * 1024;

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
    pub vmm: RwLock<Option<VirtMemManager>>,
    pub memory: Arc<VmAccounting>,
    pub default_pool: Arc<MemPool>,
    pub root_addr: usize,

    // Bootstrap registration and scheduler retirement must not sleep.
    threads: IrqTicketLock<Vec<*mut Thread>>,
    pub active_threads: AtomicUsize,
    unreaped_threads: AtomicUsize,

    pub lifecycle: IrqTicketLock<LifecycleState>,
    pub completion_waiters: TicketLock<WaiterList>,

    pub futexes: FutexTable,
}

unsafe impl Send for ProcessControlBlock {}
unsafe impl Sync for ProcessControlBlock {}

#[derive(Debug)]
struct LifecycleState {
    phase: ProcLifecycle,
    initial_thread: Option<*mut Thread>,
    notification_claim: Option<NotificationClaim>,
}

unsafe impl Send for LifecycleState {}

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

    pub const fn signal_kill(signal: ProcSignal, sender_pid: usize) -> Self {
        Self { reason: ProcTermReason::TerminatedBySignal, code: signal as u32, detail: sender_pid }
    }

    pub const fn signal_unhandled_interrupt(signal: ProcSignal, sender_pid: usize) -> Self {
        Self { reason: ProcTermReason::TerminatedBySignal, code: signal as u32, detail: sender_pid }
    }
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
    pub fn new_unregistered(mut init_table: HandleTable, name: String, credentials: Credentials) -> Result<Process, VmError> {
        let default_pool = Arc::new(MemPool::new_expandable(
            DEFAULT_PROCESS_MEMORY_LIMIT,
            DEFAULT_PROCESS_MEMORY_MAXIMUM,
            None,
        ));

        init_table.insert_at(
            HandleID(4),
            default_pool.clone(),
            AccessRights::WRITE | AccessRights::CREATE | AccessRights::MUTATE,
        );

        let vmm = VirtMemManager::try_new(&ALLOCATOR)?;
        let memory = vmm.accounting();
        let root_addr = vmm.address_space_root();

        let lifecycle = IrqTicketLock::new(LifecycleState {
            phase: ProcLifecycle::Running,
            initial_thread: None,
            notification_claim: None,
        });

        Ok(Arc::new(Self {
            proc_id: get_new_pid(),
            proc_name: name,
            credentials,

            handles: RwLock::new(init_table),
            vmm: RwLock::new(Some(vmm)),
            memory,
            default_pool,
            root_addr,

            threads: IrqTicketLock::new(Vec::new()),
            active_threads: AtomicUsize::new(0),
            unreaped_threads: AtomicUsize::new(0),

            lifecycle,
            completion_waiters: TicketLock::new(WaiterList::new()),

            futexes: FutexTable::new(),
        }))
    }

    pub fn new(init_table: HandleTable, name: String, credentials: Credentials) -> Process {
        let proc = Self::new_unregistered(init_table, name, credentials)
            .expect("cannot create kernel process address space");
        register_process(&proc);
        proc
    }

    pub fn snapshot_info(&self) -> ProcInfo {
        let phase = self.lifecycle.lock().phase;
        let (state, term_reason, term_code, term_detail) = match phase {
            ProcLifecycle::Running => (ProcState::Running, ProcTermReason::None, 0, 0),
            ProcLifecycle::Terminating(t) => (ProcState::Terminating, t.reason, t.code, t.detail),
            ProcLifecycle::Terminated(t) => (ProcState::Terminated, t.reason, t.code, t.detail),
        };

        let memory_usage = if self.proc_id == 0 { kernel_heap_allocated() } else { 
            self.vmm.read().as_ref()
                .map_or(0, |vmm| vmm.accounting().snapshot().resident_bytes)
        };

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

    fn claim_notifications(&self) -> Result<usize, InvocationError> {
        let caller = current_process().ok_or(InvocationError::InvalidHandle)?;
        if !core::ptr::eq(self, Arc::as_ptr(&caller)) {
            return Err(InvocationError::AccessDenied);
        }

        let receiver = Arc::new(NotificationReceiver::new());

        let mut handles = self.handles.write();
        let mut lifecycle = self.lifecycle.lock();
        
        if lifecycle.phase != ProcLifecycle::Running || lifecycle.notification_claim.is_some() {
            return Err(InvocationError::InvalidArgument);
        }

        let owner_handle = handles.insert(receiver.clone(), AccessRights::READ);
        lifecycle.notification_claim = Some(NotificationClaim { owner_handle, receiver });

        Ok(owner_handle.0)
    }

    fn begin_termination_locked(lifecycle: &mut LifecycleState, termination: ProcTermination) -> bool {
        if lifecycle.phase != ProcLifecycle::Running {
            return false;
        }

        lifecycle.phase = ProcLifecycle::Terminating(termination);

        if let Some(thread) = lifecycle.initial_thread.take() {
            // the scheduler retires it before running the entry point
            unsafe { enqueue_new_user_thread(thread) }
        }

        true
    }

    pub fn request_terminate(&self, termination: ProcTermination) -> bool {
        let started = {
            let mut lifecycle = self.lifecycle.lock();
            Self::begin_termination_locked(&mut lifecycle, termination)
        };

        if started {
            self.cancel_threads_for_termination();
        }

        started
    }

    fn cancel_threads_for_termination(&self) {
        for pin in self.pinned_thread_snapshot() {
            request_thread_termination(&pin);
        }
    }

    fn deliver_interrupt(&self, sender_pid: usize) {
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle.phase != ProcLifecycle::Running { return };

        if let Some(claim) = &lifecycle.notification_claim {
            let wakers = claim.receiver.post(sender_pid);
            drop(lifecycle);
            wake_all(wakers);
            return;
        }

        // default action if interrupt not handled
        let reason = ProcTermination::signal_unhandled_interrupt(
            ProcSignal::Interrupt,
            sender_pid,
        );
        Self::begin_termination_locked(&mut lifecycle, reason);
        drop(lifecycle);
        self.cancel_threads_for_termination();
    }

    pub fn close_handle(&self, id: HandleID) -> Result<(), InvocationError> {
        let mut handles = self.handles.write();
        let removed = handles.remove(id)?;
        let mut lifecycle = self.lifecycle.lock();

        let owns_claim = lifecycle.notification_claim.as_ref()
            .is_some_and(|claim| claim.owner_handle == id);

        let wakers = if owns_claim {
            let claim = lifecycle.notification_claim.take().unwrap();
            claim.receiver.release()
        } else {
            Vec::new()
        };

        drop(lifecycle);
        drop(handles);
        drop(removed);

        wake_all(wakers);
        Ok(())
    }

    pub fn finish_thread_exit(&self, thread: *mut Thread, normal_exit_code: u32) -> bool {
        if !self.unregister_thread(thread) {
            return false;
        }

        let prev = self.active_threads.fetch_sub(1, Ordering::AcqRel);
        if prev != 1 {
            return true;
        }

        let mut lc = self.lifecycle.lock();

        let termination = match lc.phase {
            ProcLifecycle::Running => ProcTermination::exited(normal_exit_code),
            ProcLifecycle::Terminating(t) => t,
            ProcLifecycle::Terminated(_) => return false,
        };

        lc.phase = ProcLifecycle::Terminated(termination);

        let notification_wakers = lc.notification_claim.take()
            .map(|claim| claim.receiver.release())
            .unwrap_or_default();

        drop(lc);
        wake_all(notification_wakers);

        unregister_process(self.proc_id);

        self.handles.write().clear();
        let wakers = self.completion_waiters.lock().take_wakers();
        wake_all(wakers);
        true
    }

    pub fn is_terminating(&self) -> bool { matches!(self.lifecycle.lock().phase, ProcLifecycle::Terminating(_) | ProcLifecycle::Terminated(_)) }

    pub fn is_terminated(&self) -> bool { matches!(self.lifecycle.lock().phase, ProcLifecycle::Terminated(_)) }

    pub fn register_thread(&self, thread: *mut Thread) {
        if thread.is_null() {
            return;
        }
        let mut threads = self.threads.lock();
        if !threads.iter().any(|&cd| cd == thread) {
            threads.push(thread);
            self.unreaped_threads.fetch_add(1, Ordering::AcqRel);
            self.active_threads.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub fn pinned_thread_snapshot(&self) -> Vec<ThreadPin> {
        let threads = self.threads.lock();

        threads.iter()
            .map(|&thread| unsafe { ThreadPin::acquire_protected(thread) })
            .collect()
    }

    pub fn unregister_thread(&self, thread: *mut Thread) -> bool {
        let mut threads = self.threads.lock();
        let old_len = threads.len();
        threads.retain(|&cd| cd != thread);
        threads.len() != old_len
    }

    pub unsafe fn install_initial_thread(&self, thread: *mut Thread) {
        assert!(!thread.is_null());

        let mut lifecycle = self.lifecycle.lock();
        assert!(matches!(lifecycle.phase, ProcLifecycle::Running));
        assert!(lifecycle.initial_thread.is_none());
        lifecycle.initial_thread = Some(thread);
    }

    pub fn resume_initial_thread(&self) {
        let mut lifecycle = self.lifecycle.lock();
        if !matches!(lifecycle.phase, ProcLifecycle::Running) { return };

        if let Some(thread) = lifecycle.initial_thread.take() {
            unsafe { enqueue_new_user_thread(thread); }
        }
    }

    pub fn finish_thread_reap(&self) {
        let prev = self.unreaped_threads.fetch_sub(1, Ordering::AcqRel);
        assert!(prev != 0, "unregistered thread reached the reaper");

        if prev == 1 && self.proc_id != 0 {
            debug_assert!(self.is_terminated());

            let addr_space = self.vmm.write().take();
            drop(addr_space);
        }
    }
}

#[async_trait]
impl KernelObject for ProcessControlBlock {
    fn type_name(&self) -> &'static str { "Process" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights, ctx: InvokeContext<'_>) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Proc(ProcOp::Resume) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.resume_initial_thread();
                Ok(0)
            },
            Invocation::Proc(ProcOp::Terminate { reason }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.request_terminate(ProcTermination::terminated(reason));
                Ok(0)
            },
            Invocation::Proc(ProcOp::GetInfo { info_ptr }) => {
                calling_rights.err_if_no(AccessRights::READ)?;
                self.info(info_ptr as *mut ProcInfo)
            },
            Invocation::Proc(ProcOp::Unmap { vaddr, len }) => {
                let mut slot = self.vmm.write();
                let vmm = slot.as_mut().ok_or(InvocationError::InvalidHandle)?;
                vmm.unmap_range(vaddr, len)?;
                Ok(0)
            },
            Invocation::Proc(ProcOp::SpawnThread { entry, stack_top, arg, priority }) => {
                let priority = ThreadPriority::from(priority);
                let proc = current_process().ok_or(InvocationError::ThreadSpawnFail)?;

                let thread = create_user_tcb(entry, stack_top, arg, priority, proc.clone())?;

                let object = unsafe { ThreadObject::attach(thread) };
                let id = self.handles.write().insert(object, AccessRights::all());

                unsafe { enqueue_new_user_thread(thread) };
                Ok(id.0)
            },
            Invocation::Wait(WaitOp::One(signal)) => ctx.wait(ObjectWaitFuture::new(self, signal)).await,
            Invocation::Wait(WaitOp::Many { items_ptr, count }) => {
                if count == 0 || count > 64 {
                    return Err(InvocationError::InvalidArgument);
                }
                ctx.wait(WaitManyFuture { process: self, items_ptr, count, waiter: AsyncWaiter::new() }).await
            },
            Invocation::Proc(ProcOp::SetFsBase { fs_base }) => {
                let current_thread = current_core_mut().scheduler.get_current_thread();
                if current_thread.is_null() {
                    return Err(InvocationError::InvalidHandle);
                }
                unsafe {
                    (*current_thread).fs_base = fs_base;
                    set_user_thread_pointer(fs_base);
                }
                Ok(0)
            },
            Invocation::Proc(ProcOp::InsertHandle { source_handle, rights }) => {
                calling_rights.err_if_no(AccessRights::MUTATE)?;
                let caller = current_process().ok_or(InvocationError::InvalidHandle)?;
                let obj = caller.handles.read().resolve(source_handle, rights)?;
                let new_handle_id = self.handles.write().insert(obj, rights);
                Ok(new_handle_id.0)
            },
            Invocation::Proc(ProcOp::Mprotect { vaddr, len, prot }) => {
                let perms = VmPermissions::from_user_vm_flags(prot)?;
                let mut slot = self.vmm.write();
                let vmm = slot.as_mut().ok_or(InvocationError::InvalidHandle)?;
                vmm.protect_range(vaddr, len, perms)?;
                Ok(0)
            },
            Invocation::Proc(ProcOp::SendSignal { code }) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                let sender = current_process().ok_or(InvocationError::InvalidHandle)?;
                let signal = ProcSignal::from_raw(code).ok_or(InvocationError::InvalidArgument)?;
                match signal {
                    ProcSignal::Kill => {
                        self.request_terminate(ProcTermination::signal_kill(signal, sender.proc_id));
                    },
                    ProcSignal::Interrupt => self.deliver_interrupt(sender.proc_id),
                }
                Ok(0)
            },
            Invocation::Proc(ProcOp::ClaimNotifications) => {
                calling_rights.err_if_no(AccessRights::WRITE)?;
                self.claim_notifications()
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
