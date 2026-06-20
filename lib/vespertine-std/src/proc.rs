extern crate alloc;
use alloc::vec::Vec;
use vespertine_abi::{AccessRights, HandleID, Invocation, ProcOp, Signal, WaitItem, WaitOp};
use vespertine_rt::syscall::sys_invoke;

use crate::{Error, env};

pub fn push_handle(
    target: HandleID,
    handle: HandleID,
    rights: AccessRights,
) -> Result<HandleID, Error> {
    let op = ProcOp::InsertHandle {
        source_handle: handle,
        rights,
    };
    let id = sys_invoke(target, &Invocation::Proc(op)).map_err(Error::from)?;
    Ok(HandleID(id))
}

pub struct Waiter {
    items: Vec<WaitItem>,
}

impl Waiter {
    pub fn new() -> Self {
        Waiter { items: Vec::new() }
    }

    pub fn readable(mut self, handle: HandleID) -> Self {
        self.items.push(WaitItem {
            handle,
            signal: Signal::READABLE,
            pending: Signal(0),
        });
        self
    }

    pub fn writeable(mut self, handle: HandleID) -> Self {
        self.items.push(WaitItem {
            handle,
            signal: Signal::WRITABLE,
            pending: Signal(0),
        });
        self
    }

    pub fn wait(&mut self) -> Result<(), Error> {
        let op = WaitOp::Many {
            items_ptr: self.items.as_mut_ptr() as usize,
            count: self.items.len(),
        };
        sys_invoke(env::self_handle(), &Invocation::Wait(op)).map_err(Error::from)?;
        Ok(())
    }

    pub fn ready(&self, idx: usize) -> bool {
        self.items
            .get(idx)
            .map(|item| item.pending.contains(item.signal))
            .unwrap_or(false)
    }

    pub fn ready_with(&self, idx: usize, signal: Signal) -> bool {
        self.items
            .get(idx)
            .map(|item| item.pending.contains(signal))
            .unwrap_or(false)
    }

    pub fn clear(&mut self) {
        for item in &mut self.items {
            item.pending = Signal(0);
        }
    }
}
