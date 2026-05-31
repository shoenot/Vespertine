use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::future::poll_fn;
use core::ptr::addr_of;
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};
use core::task::{
    Context,
    Poll,
};

use async_trait::async_trait;
use vespertine_abi::op::ProcOp;
use vespertine_abi::{
    AccessRights,
    HandleID,
    Invocation,
    ProcStatus,
    Signal,
    WaitItem,
    WaitOp,
};

use crate::arch::x86_64::task::syscall::{
    safe_copy_from,
    safe_copy_to,
};
use crate::core::object::handle::HandleTable;
use crate::core::object::invoke::InvocationError;
use crate::core::object::models::socket::SocketEndpoint;
use crate::core::object::models::thread::Thread;
use crate::core::object::obj::KernelObject;
use crate::core::sync::RwLock;
use crate::core::thread::dispatch::spawn_user_thread;
use crate::core::thread::get_current_process;
use crate::core::thread::priority::ThreadPriority;
use crate::memory::ALLOCATOR;
use crate::memory::vmm::VirtMemManager;

pub static GLOBAL_PID: AtomicUsize = AtomicUsize::new(0);

pub fn get_new_pid() -> usize { GLOBAL_PID.fetch_add(1, core::sync::atomic::Ordering::Relaxed) }

pub type Process = Arc<ProcessControlBlock>;

#[repr(C)]
#[derive(Debug)]
pub struct ProcessControlBlock {
    pub proc_id: usize,
    pub proc_handles: RwLock<HandleTable>,
    pub vmm: RwLock<VirtMemManager>,
    pub active_threads: AtomicUsize,
    pub is_terminated: AtomicBool,
}

impl ProcessControlBlock {
    pub fn new(init_table: HandleTable) -> Process {
        Arc::new(Self {
            proc_id: get_new_pid(),
            proc_handles: RwLock::new(init_table),
            vmm: RwLock::new(VirtMemManager::new(&ALLOCATOR)),
            active_threads: AtomicUsize::new(0),
            is_terminated: AtomicBool::new(false),
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

    fn wait_many_async(&self, items_ptr: *mut WaitItem, count: usize, cx: &mut Context<'_>) -> Poll<Result<usize, InvocationError>> {
        let mut items = vec![WaitItem { handle: HandleID(0), signal: Signal(0), pending: Signal(0) }; count];

        if !safe_copy_from(items.as_mut_ptr() as *mut u8, items_ptr as *const u8, count * size_of::<WaitItem>()) {
            return Poll::Ready(Err(InvocationError::InvalidPointer));
        }

        let mut endpoints: Vec<Arc<SocketEndpoint>> = Vec::with_capacity(count);

        {
            let table = self.proc_handles.read();
            for item in &items {
                let entry = table.resolve_entry(item.handle, AccessRights::READ)?;
                if entry.object.type_name() != "Socket" {
                    return Poll::Ready(Err(InvocationError::UnsupportedOperation));
                }
                let ep = unsafe {
                    let raw_fat = Arc::into_raw(entry.object.clone());
                    let raw_thin = raw_fat as *const () as *const SocketEndpoint;
                    Arc::from_raw(raw_thin)
                };
                endpoints.push(ep);
            }
        }

        // poll each endpoint for satisfied signals
        let mut any_ready = false;
        for (i, ep) in endpoints.iter().enumerate() {
            items[i].pending = Signal(0);
            let sig = items[i].signal;

            if sig.contains(Signal::READABLE) {
                let bus = ep.read_bus.buffer.lock();
                if !bus.is_empty() || ep.read_bus.is_closed.load(Ordering::SeqCst) {
                    items[i].pending = items[i].pending | Signal::READABLE;
                    any_ready = true;
                }
            }

            if sig.contains(Signal::PEER_CLOSED) {
                if ep.read_bus.is_closed.load(Ordering::SeqCst) {
                    items[i].pending = items[i].pending | Signal::PEER_CLOSED;
                    any_ready = true;
                }
            }
        }

        if any_ready {
            safe_copy_to(items_ptr as *mut u8, items.as_ptr() as *const u8, count * size_of::<WaitItem>());
            return Poll::Ready(Ok(0));
        }

        // if none ready, register waker across all socket buses
        for ep in &endpoints {
            *ep.read_bus.read_waker.lock() = Some(cx.waker().clone());
        }

        Poll::Pending
    }
}

#[async_trait]
impl KernelObject for ProcessControlBlock {
    fn type_name(&self) -> &'static str { "Process" }

    async fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Proc(ProcOp::Kill) => {
                self.is_terminated.store(true, Ordering::SeqCst);
                Ok(0)
            }
            Invocation::Proc(ProcOp::GetStatus { status_ptr }) => self.status(status_ptr as *mut ProcStatus),
            Invocation::Proc(ProcOp::Unmap { vaddr, len }) => {
                self.vmm.write().munmap(vaddr, len).map(|_| 0).map_err(|_| InvocationError::InvalidArgument)
            }
            Invocation::Proc(ProcOp::SpawnThread { entry, stack_top, arg, priority }) => {
                let tp = ThreadPriority::from(priority);
                let proc = get_current_process().ok_or(InvocationError::ThreadSpawnFail)?;
                let thread = spawn_user_thread(entry, stack_top, arg, tp, proc.clone());
                self.active_threads.fetch_add(1, Ordering::Relaxed);
                let obj = Arc::new(Thread { tcb: thread });
                let id = self.proc_handles.write().insert(obj, AccessRights::all());
                Ok(id.0)
            }
            Invocation::Wait(WaitOp::Many { items_ptr, count }) => {
                if count == 0 || count > 64 {
                    return Err(InvocationError::InvalidArgument);
                }
                poll_fn(move |cx| self.wait_many_async(items_ptr as *mut WaitItem, count, cx)).await
            }
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}
