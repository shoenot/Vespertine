use core::{cell::UnsafeCell, sync::atomic::{AtomicUsize, Ordering}};

use alloc::sync::Arc;
use alloc::format;
use crate::{arch::x86_64::task::syscall::safe_copy_to, core::object::vfs::mount_kernel_dir};

use mnemosyne_abi::{HandleID, AccessRights};
use crate::core::object::invoke::{Invocation, InvocationError};
use crate::core::object::obj::KernelObject ;
use crate::core::object::vfs::{kernel_register_obj, kernel_invoke};
use crate::core::sync::Semaphore;
use mnemosyne_abi::op::DirectoryOp;
use mnemosyne_abi::op::ChannelOp;

const CAPACITY: usize = 4;
const MASK: usize = CAPACITY - 1;
static CHAN_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MessageSlot {
    pub data: [u8; 64],
    pub len: usize,
}

#[repr(C)]
#[derive(Debug)]
pub struct SpscQueue { 
    pub storage: UnsafeCell<[MessageSlot; CAPACITY]>,
    pub head: AtomicUsize,
    pub tail: AtomicUsize,
}

unsafe impl Sync for SpscQueue {}

impl SpscQueue {
    pub const fn new() -> Self {
        SpscQueue { 
            storage: UnsafeCell::new([MessageSlot { data: [0; 64], len: 0 }; CAPACITY]),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn push(&self, data: [u8; 64], len: usize) -> Result<(), ()> {
        let current_head = self.head.load(Ordering::Relaxed);
        let current_tail = self.tail.load(Ordering::Acquire);

        if ((current_head + 1) & 3) == (current_tail & 3) {
            return Err(()); // buffer full
        }

        let slot_idx = current_head & 3;
        
        unsafe {
            (*self.storage.get())[slot_idx] = MessageSlot { data, len };
        }

        self.head.store(current_head + 1, Ordering::Release);
        Ok(())
    }

    pub fn pop(&self) -> Option<MessageSlot> {
        let current_tail = self.tail.load(Ordering::Relaxed);
        let current_head = self.head.load(Ordering::Acquire);

        if current_head == current_tail {
            return None; // buffer empty
        }

        let slot_idx = current_tail & 3;
        
        let ret = unsafe {
            (*self.storage.get())[slot_idx]
        };

        self.tail.store(current_tail + 1, Ordering::Release);
        Some(ret)
    }
}

#[derive(Debug)]
pub struct ChannelState {
    pub queue_a_to_b: SpscQueue,
    pub queue_b_to_a: SpscQueue,
    pub wait_a: Semaphore,
    pub wait_b: Semaphore,
}

#[derive(Copy, Clone, Debug)]
pub enum ChannelSide {
    SideA,
    SideB,
}

impl ChannelState {
    pub fn new() -> Self {
        Self { 
            queue_a_to_b: SpscQueue::new(), 
            queue_b_to_a: SpscQueue::new(), 
            wait_a: Semaphore::new(0),
            wait_b: Semaphore::new(0), 
        }
    }

    pub fn send(&self, side: ChannelSide, data: [u8; 64], len: usize) -> Result<(), ()> {
        match side {
            ChannelSide::SideA => {
                self.queue_a_to_b.push(data, len)?;
                self.wait_b.signal();
            },
            ChannelSide::SideB => {
                self.queue_b_to_a.push(data, len)?;
                self.wait_a.signal();
            },
        }
        Ok(())
    }

    pub fn recieve(&self, side: ChannelSide) -> MessageSlot {
        match side {
            ChannelSide::SideA => {
                loop {
                    if let Some(slot) = self.queue_b_to_a.pop() {
                        return slot 
                    }

                    self.wait_a.wait();
                }
            },
            ChannelSide::SideB => {
                loop {
                    if let Some(slot) = self.queue_a_to_b.pop() {
                        return slot 
                    }

                    self.wait_b.wait();
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Channel {
    pub state: Arc<ChannelState>,
    pub side: ChannelSide,
}

impl Channel {
    pub fn new_pair() -> (Self, Self) {
        let state = Arc::new(ChannelState::new());

        let endpoint_a = Self {
            state: state.clone(),
            side: ChannelSide::SideA,
        };

        let endpoint_b = Self {
            state: state.clone(),
            side: ChannelSide::SideB,
        };

        (endpoint_a, endpoint_b)
    }
}

impl KernelObject for Channel {
    fn invoke(&self, invocation: Invocation, _calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Channel(ChannelOp::PushSmall { data, len }) => {
                if len as usize > data.len() { return Err(InvocationError::InvalidArgument) };
                match self.state.send(self.side, data, len as usize) {
                    Ok(_) => Ok(0),
                    Err(_) => Err(InvocationError::BufferFull),
                }
            },
            Invocation::Channel(ChannelOp::Pull { buffer_ptr }) => {
                let mut slot = self.state.recieve(self.side);

                if !safe_copy_to(buffer_ptr, slot.data.as_mut_ptr(), slot.len) { 
                    return Err(InvocationError::InvalidArgument); 
                }

                Ok(slot.len)
            }, 
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }

    fn type_name(&self) -> &'static str {
        "Channel"
    }
}

pub fn link_chan(handle: HandleID) {
    let obj_root = kernel_invoke(
        HandleID(0),
        Invocation::Directory(DirectoryOp::Lookup { name: "Objects".as_ptr(), name_len: "Objects".len() })
    ).expect("Obj dir not mounted.");

    let chan_root = kernel_invoke(
        HandleID(obj_root),
        Invocation::Directory(DirectoryOp::Lookup { name: "Channels".as_ptr(), name_len: "Channels".len() })
    ).expect("Chan root not mounted.");

    mount_kernel_dir(
        &format!("Chan{}", CHAN_COUNTER.fetch_add(1, Ordering::Relaxed)), 
        handle, 
        HandleID(chan_root)
    );
}

pub fn init_ipc_pipeline() -> (HandleID, HandleID) {
    let (endpoint_a, endpoint_b) = Channel::new_pair();
    let obj_a = Arc::new(endpoint_a);
    let obj_b = Arc::new(endpoint_b);
    let handle_a = kernel_register_obj(obj_a, AccessRights::READ | AccessRights::WRITE);
    let handle_b = kernel_register_obj(obj_b, AccessRights::READ | AccessRights::WRITE);
    link_chan(handle_a);
    link_chan(handle_b);
    (handle_a, handle_b)
}
