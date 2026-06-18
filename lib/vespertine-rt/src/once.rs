use core::{
    cell::UnsafeCell,
    convert::Infallible,
    mem::MaybeUninit,
    sync::atomic::{AtomicU32, Ordering},
};

use crate::syscall::{sys_futex_wait, sys_futex_wake};

const UNINITIALIZED: u32 = 0;
const INITIALIZING: u32 = 1;
const INITIALIZED: u32 = 2;

pub struct OnceCell<T> {
    state: AtomicU32,
    value: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Send> Send for OnceCell<T> {}
unsafe impl<T: Send + Sync> Sync for OnceCell<T> {}

impl<T> OnceCell<T> {
    pub const fn new() -> Self {
        Self {
            state: AtomicU32::new(UNINITIALIZED),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == INITIALIZED {
            Some(unsafe { (*self.value.get()).assume_init_ref() })
        } else {
            None
        }
    }

    pub fn get_or_init<F>(&self, initialize: F) -> &T
    where
        F: FnOnce() -> T,
    {
        match self.get_or_try_init::<_, Infallible>(|| Ok(initialize())) {
            Ok(value) => value,
            Err(error) => match error {},
        }
    }

    pub fn get_or_try_init<F, E>(&self, initialize: F) -> Result<&T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        if let Some(value) = self.get() {
            return Ok(value);
        }

        let mut initialize = Some(initialize);

        loop {
            match self.state.compare_exchange(
                UNINITIALIZED,
                INITIALIZING,
                Ordering::Acquire,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let result = initialize
                        .take()
                        .expect("initializer called more than once")(
                    );

                    match result {
                        Ok(value) => {
                            unsafe {
                                (*self.value.get()).write(value);
                            }

                            self.state.store(INITIALIZED, Ordering::Release);
                            Self::wake_waiters(&self.state);
                            return Ok(unsafe { (*self.value.get()).assume_init_ref() });
                        }

                        Err(error) => {
                            self.state.store(UNINITIALIZED, Ordering::Release);
                            Self::wake_waiters(&self.state);
                            return Err(error);
                        }
                    }
                }

                Err(INITIALIZED) => {
                    return Ok(unsafe { (*self.value.get()).assume_init_ref() });
                }

                Err(INITIALIZING) => {
                    Self::wait_while_initializing(&self.state);
                }

                Err(_) => unreachable!(),
            }
        }
    }

    fn wait_while_initializing(state: &AtomicU32) {
        while state.load(Ordering::Acquire) == INITIALIZING {
            let address = state as *const AtomicU32 as usize;
            sys_futex_wait(address, INITIALIZING);
        }
    }

    fn wake_waiters(state: &AtomicU32) {
        let address = state as *const AtomicU32 as usize;
        sys_futex_wake(address, usize::MAX);
    }
}

impl<T> Drop for OnceCell<T> {
    fn drop(&mut self) {
        if *self.state.get_mut() == INITIALIZED {
            unsafe {
                self.value.get_mut().assume_init_drop();
            }
        }
    }
}
