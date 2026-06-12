use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use vespertine_common::lock::TicketLock;
use core::future::Future;
use core::pin::Pin;
use core::ptr::addr_of;
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};
use core::task::{
    Context,
    Poll, Waker,
};

use async_trait::async_trait;
use vespertine_abi::op::ProcOp;
use vespertine_abi::{
    AccessRights, HandleID, Invocation, ProcStatus, ProcessExitInfo, ProcessExitKind, Signal, WaitItem, WaitOp
};

use crate::arch::get_core_data;
use crate::arch::x86_64::task::syscall::{
    safe_copy_from,
    safe_copy_to,
};
use crate::core::asynchronous::waiter::{AsyncWaiter, WaiterList, wake_all};
use crate::core::object::handle::HandleTable;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::socket::{
    SocketEndpoint,
};
use crate::core::object::models::thread::Thread;
use crate::core::object::obj::{KernelObject, ObjectWaitFuture, matching_signals};
use crate::core::sync::RwLock;
use crate::core::thread::dispatch::spawn_user_thread;
use crate::core::thread::get_current_process;
use crate::core::thread::priority::ThreadPriority;
use crate::core::thread::wait::WaitQueue;
use crate::memory::ALLOCATOR;
use crate::memory::vmm::VirtMemManager;
use crate::util::write_to_msr;

pub static GLOBAL_PID: AtomicUsize = AtomicUsize::new(0);

pub fn get_new_pid() -> usize { GLOBAL_PID.fetch_add(1, core::sync::atomic::Ordering::Relaxed) }

pub type Process = Arc<ProcessControlBlock>;

#[repr(C)]
#[derive(Debug)]
pub struct ProcessControlBlock {
    pub proc_id: usize,
    pub proc_handles: RwLock<HandleTable>,
    pub vmm: RwLock<VirtMemManager>,
    pub pml4_addr: usize,
    pub active_threads: AtomicUsize,
    pub is_terminated: AtomicBool,
    pub futexes: RwLock<BTreeMap<usize, WaitQueue>>,
    pub completion_waiters: TicketLock<WaiterList>,
    pub exit_info: RwLock<ProcessExitInfo>,
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
        let table = self.process.proc_handles.read();
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
    pub fn new(init_table: HandleTable) -> Process {
        let vmm = VirtMemManager::new(&ALLOCATOR);
        let pml4_addr = vmm.get_pml4_addr();
        Arc::new(Self {
            proc_id: get_new_pid(),
            proc_handles: RwLock::new(init_table),
            vmm: RwLock::new(vmm),
            pml4_addr,
            active_threads: AtomicUsize::new(0),
            is_terminated: AtomicBool::new(false),
            futexes: RwLock::new(BTreeMap::new()),
            completion_waiters: TicketLock::new(WaiterList::new()),
            exit_info: RwLock::new(ProcessExitInfo::running()),
        })
    }

    pub fn status(&self, ptr: *mut ProcStatus) -> Result<usize, InvocationError> {
        let proc_status = ProcStatus {
            pid: self.proc_id,
            active_threads: self.active_threads.load(Ordering::Relaxed),
            is_terminated: self.is_terminated.load(Ordering::Relaxed),
            memory_usage: self.vmm.read().get_total_allocated_size(),
        };
        let src_ptr = addr_of!(proc_status) as *const u8;
        safe_copy_to(ptr as *mut u8, src_ptr, size_of::<ProcStatus>());
        Ok(0)
    }


    pub fn complete(&self, exit_info: ProcessExitInfo) -> bool {
        {
            let mut stored = self.exit_info.write();
    
            if stored.kind != ProcessExitKind::Running {
                return false;
            }
    
            *stored = exit_info;
        }
    
        self.is_terminated.store(true, Ordering::Release);
        self.proc_handles.write().clear();
    
        let wakers = self.completion_waiters.lock().take_wakers();
        wake_all(wakers);
    
        true
    }


    pub fn get_exit_info(&self, ptr: *mut ProcessExitInfo) -> Result<usize, InvocationError> {
        let exit_info = *self.exit_info.read();

        if !safe_copy_to(
            ptr as *mut u8, 
            &exit_info as *const ProcessExitInfo as *const u8, 
            size_of::<ProcessExitInfo>()
        ) {
            return Err(InvocationError::InvalidPointer);
        }

        Ok(0)
    }
}

#[async_trait]
impl KernelObject for ProcessControlBlock {
    fn type_name(&self) -> &'static str { "Process" }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Proc(ProcOp::Kill) => {
                self.complete(ProcessExitInfo::killed(0));
                Ok(0)
            }
            Invocation::Proc(ProcOp::GetStatus { status_ptr }) => self.status(status_ptr as *mut ProcStatus),
            Invocation::Proc(ProcOp::GetExitInfo { info_ptr }) => self.get_exit_info(info_ptr as *mut ProcessExitInfo),
            Invocation::Proc(ProcOp::Unmap { vaddr, len }) => {
                self.vmm.write().munmap(vaddr, len).map(|_| 0).map_err(|_| InvocationError::InvalidArgument)
            }
            Invocation::Proc(ProcOp::SpawnThread { entry, stack_top, arg, priority }) => {
                let tp = ThreadPriority::from(priority);
                let proc = get_current_process().ok_or(InvocationError::ThreadSpawnFail)?;
                let thread = spawn_user_thread(entry, stack_top, arg, tp, proc.clone());
                let obj = Arc::new(Thread { tcb: thread });
                let id = self.proc_handles.write().insert(obj, AccessRights::all());
                Ok(id.0)
            }
            Invocation::Wait(WaitOp::One(signal)) => {
                ObjectWaitFuture::new(self, signal).await
            },
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
                if !calling_rights.contains(AccessRights::MUTATE) {
                    return Err(InvocationError::AccessDenied);
                }
                let caller = get_current_process().ok_or(InvocationError::InvalidHandle)?;
                let obj = caller.proc_handles.read().resolve(source_handle, rights)?;
                let new_handle_id = self.proc_handles.write().insert(obj, rights);
                Ok(new_handle_id.0)
            }
            Invocation::Proc(ProcOp::Mprotect { vaddr, len, prot }) => {
                self.vmm.write().mprotect(vaddr, len, prot).map(|_| 0).map_err(|_| InvocationError::InvalidArgument)
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn current_signals(&self) -> Signal {
        if self.is_terminated.load(Ordering::Acquire) { Signal::TERMINATED }
        else { Signal(0) }
    }

    fn register_waiter(&self, requested: Signal, waiter: &Arc<AsyncWaiter>, waker: &Waker) -> Result<(), InvocationError> {
        if requested != Signal::TERMINATED { 
            return Err(InvocationError::UnsupportedOperation);
        }

        self.completion_waiters.lock().register(waiter, waker);
        Ok(())
    }
}
