use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::Deref;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

pub struct KernelOnceCell<T> {
    state: AtomicUsize,
    value: UnsafeCell<MaybeUninit<T>>,
}

const UNINITIATED: usize = 0;
const RUNNING: usize = 1;
const READY: usize = 2;

unsafe impl<T: Sync + Send> Sync for KernelOnceCell<T> {}

impl<T> KernelOnceCell<T> {
    pub const fn new() -> Self { Self { state: AtomicUsize::new(UNINITIATED), value: UnsafeCell::new(MaybeUninit::uninit()) } }

    pub fn get_or_init<F>(&self, f: F) -> &T
    where
        F: FnOnce() -> T,
    {
        if self.state.load(Ordering::Acquire) == READY {
            return unsafe { (*self.value.get()).assume_init_ref() };
        }

        loop {
            match self.state.compare_exchange(UNINITIATED, RUNNING, Ordering::Acquire, Ordering::Acquire) {
                Ok(_) => {
                    unsafe { (*self.value.get()).write(f()) };
                    self.state.store(READY, Ordering::Release);
                    return unsafe { (*self.value.get()).assume_init_ref() };
                }
                Err(s) => {
                    if s == READY {
                        // someone else initiated it in the middle
                        return unsafe { (*self.value.get()).assume_init_ref() };
                    }
                }
            }
        }
    }

    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == READY {
            unsafe { Some((*self.value.get()).assume_init_ref()) }
        } else {
            None
        }	
    }
}

impl<T> Deref for KernelOnceCell<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target { unsafe { &(*self.value.get()).assume_init_ref() } }
}
