use core::{
    cell::UnsafeCell, hint::spin_loop, ops::Deref, sync::atomic::{
        AtomicUsize,
        Ordering,
    }
};

pub struct KernelOnceCell<T> {
    state: AtomicUsize,
    value: UnsafeCell<Option<T>>,
}

const UNINITIATED: usize = 0;
const RUNNING: usize = 1;
const READY: usize = 2;

unsafe impl<T: Sync + Send> Sync for KernelOnceCell<T> {}

impl<T> KernelOnceCell<T> {
    pub const fn new() -> Self {
        Self {
            state: AtomicUsize::new(UNINITIATED),
            value: UnsafeCell::new(None),
        }
    }

    pub fn get_or_init<F>(&self, f: F) -> &T
    where F: FnOnce() -> T {
        if self.state.load(Ordering::Acquire) == READY {
            return unsafe { (*self.value.get()).as_ref().unwrap_unchecked() }
        }

        loop {
            match self.state.compare_exchange(UNINITIATED, RUNNING, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => {
                    unsafe { *self.value.get() = Some(f()) };
                    self.state.store(READY, Ordering::Release);
                    return unsafe { (*self.value.get()).as_ref().unwrap_unchecked() };
                },
                Err(s) => if s == READY {
                    // someone else initiated it in the middle
                    return unsafe { (*self.value.get()).as_ref().unwrap_unchecked() };
                },
                _ => spin_loop(),
            }
        }
    }
}

impl<T> Deref for KernelOnceCell<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &(*self.value.get()).expect("Derefing an uninitialized KernelOnceCell") } 
    }
}
