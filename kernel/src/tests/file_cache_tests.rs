use alloc::boxed::Box;
use alloc::sync::Arc;
use core::future::{Future, poll_fn};
use core::pin::{Pin, pin};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::task::{Poll, Waker};

use async_trait::async_trait;

use crate::executor::waiter::{TerminationToken, wake_all};
use crate::memory::vmo::FileVmo;
use crate::memory::{DIRECT_MAP_OFFSET, PAGE_SIZE_4K as PAGE};
use crate::object::invoke::{InvocationError, InvokeContext};
use crate::storage::fs::{VfsNode, VfsNodeType, VfsReadError};
use crate::sync::TicketLock;

const PATTERN: u8 = 0xa7;

// Each event has one waiting task. Once signaled, it stays signaled.
#[derive(Debug)]
struct Event {
    state: TicketLock<(bool, Option<Waker>)>,
}

impl Event {
    fn new() -> Self {
        Self { state: TicketLock::new((false, None)) }
    }

    async fn wait(&self) {
        poll_fn(|cx| {
            let mut state = self.state.lock();
            if state.0 {
                return Poll::Ready(());
            }

            state.1 = Some(cx.waker().clone());
            Poll::Pending
        }).await
    }

    fn signal(&self) {
        let waker = {
            let mut state = self.state.lock();
            state.0 = true;
            state.1.take()
        };

        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

#[derive(Debug)]
struct GatedFile {
    size: AtomicUsize,
    reads: AtomicUsize,
    fail_once: AtomicBool,
    started: Event,
    release: Event,
}

#[async_trait]
impl VfsNode for GatedFile {
    async fn read_at_phys(
        &self,
        offset: usize,
        dest_phys: usize,
        len: usize,
    ) -> Result<usize, VfsReadError> {
        assert_eq!(offset, 0);
        assert_eq!(len, PAGE);

        self.reads.fetch_add(1, Ordering::AcqRel);
        self.started.signal();
        self.release.wait().await;

        if self.fail_once.swap(false, Ordering::AcqRel) {
            return Err(VfsReadError::Io);
        }

        // FileVmo owns this frame until read_at_phys returns.
        let dst = (dest_phys + *DIRECT_MAP_OFFSET) as *mut u8;
        unsafe { dst.write_bytes(PATTERN, len) };
        Ok(len)
    }

    fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    fn node_type(&self) -> VfsNodeType {
        VfsNodeType::File
    }
}

fn fixture(fail_once: bool) -> (Arc<GatedFile>, Arc<FileVmo>) {
    let file = Arc::new(GatedFile {
        size: AtomicUsize::new(PAGE),
        reads: AtomicUsize::new(0),
        fail_once: AtomicBool::new(fail_once),
        started: Event::new(),
        release: Event::new(),
    });

    let node = Arc::downgrade(&file);
    let cache = FileVmo::new(node);
    (file, cache)
}

async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await
}

fn assert_filled(cache: &FileVmo) {
    let frame = cache.cached_frame(0).expect("fill did not publish a frame");
    // The fixture has no concurrent writer at this point.
    let bytes = unsafe {
        core::slice::from_raw_parts(frame.virt() as *const u8, PAGE)
    };
    assert!(bytes.iter().all(|&byte| byte == PATTERN));
}

async fn test_fill_survives_first_waiter() {
    let (file, cache) = fixture(false);
    let fill = {
        let _io = cache.io_lock.lock().await;
        let fill = cache.start_or_join_fill_locked(0).unwrap();
        let joined = cache.start_or_join_fill_locked(0).unwrap();
        assert!(Arc::ptr_eq(&fill, &joined));
        fill
    };

    let token = TerminationToken::new();
    let ctx = InvokeContext::for_thread(&token);
    let mut first = pin!(ctx.wait(fill.wait()));
    assert_eq!(poll_once(first.as_mut()).await, Poll::Pending);

    file.started.wait().await;
    let joined = cache.pending_fill(0).expect("running fill missing");
    assert!(Arc::ptr_eq(&fill, &joined));
    let mut second = pin!(joined.wait());
    assert_eq!(poll_once(second.as_mut()).await, Poll::Pending);

    wake_all(token.request_and_take_wakers());
    assert_eq!(first.await, Err(InvocationError::Terminating));

    // A has unwound while the backend is still gated. B is still waiting.
    assert!(cache.cached_frame(0).is_none());
    assert_eq!(poll_once(second.as_mut()).await, Poll::Pending);

    file.release.signal();
    assert_eq!(second.await, Ok(()));
    assert_filled(&cache);
    assert!(cache.pending_fill(0).is_none());
    assert_eq!(file.reads.load(Ordering::Acquire), 1);
}

async fn test_shrink_before_fill() {
    let (file, cache) = fixture(false);

    // Open the backend gate so an unexpected read can finish and be counted.
    file.release.signal();
    let fill = {
        let _io = cache.io_lock.lock().await;
        let fill = cache.start_or_join_fill_locked(0).unwrap();

        file.size.store(0, Ordering::Release);
        cache.truncate_cached_locked(0, None);
        fill
    };

    assert_eq!(fill.wait().await, Ok(()));
    assert_eq!(file.reads.load(Ordering::Acquire), 0);
    assert!(cache.cached_frame(0).is_none());
    assert!(cache.pending_fill(0).is_none());
}

async fn test_fill_before_shrink() {
    let (file, cache) = fixture(false);
    let fill = {
        let _io = cache.io_lock.lock().await;
        cache.start_or_join_fill_locked(0).unwrap()
    };
    file.started.wait().await;

    let mut shrink = pin!(async {
        let _io = cache.io_lock.lock().await;

        // The worker publishes the page before releasing io_lock.
        assert!(cache.cached_frame(0).is_some());
        file.size.store(0, Ordering::Release);
        cache.truncate_cached_locked(0, None);
    });

    assert_eq!(poll_once(shrink.as_mut()).await, Poll::Pending);
    file.release.signal();
    shrink.await;

    assert_eq!(fill.wait().await, Ok(()));
    assert!(cache.cached_frame(0).is_none());
    assert!(cache.pending_fill(0).is_none());
    assert_eq!(file.reads.load(Ordering::Acquire), 1);
}

async fn test_failed_fill_can_retry() {
    let (file, cache) = fixture(true);
    let fill = {
        let _io = cache.io_lock.lock().await;
        cache.start_or_join_fill_locked(0).unwrap()
    };
    file.started.wait().await;

    let mut first = pin!(fill.wait());
    let mut second = pin!(fill.wait());
    assert_eq!(poll_once(first.as_mut()).await, Poll::Pending);
    assert_eq!(poll_once(second.as_mut()).await, Poll::Pending);

    file.release.signal();
    assert_eq!(first.await, Err(InvocationError::Io));
    assert_eq!(second.await, Err(InvocationError::Io));
    assert!(cache.cached_frame(0).is_none());
    assert!(cache.pending_fill(0).is_none());
    assert_eq!(file.reads.load(Ordering::Acquire), 1);

    let retry = {
        let _io = cache.io_lock.lock().await;
        cache.start_or_join_fill_locked(0).unwrap()
    };
    assert!(!Arc::ptr_eq(&fill, &retry));
    assert_eq!(retry.wait().await, Ok(()));
    assert_filled(&cache);
    assert_eq!(file.reads.load(Ordering::Acquire), 2);
}

pub(super) async fn run() {
    crate::klogln!(Info, "Test/FileCache", "Fill survives first waiter");
    test_fill_survives_first_waiter().await;
    crate::klogln!(Info, "Test/FileCache", "Shrink before fill");
    test_shrink_before_fill().await;
    crate::klogln!(Info, "Test/FileCache", "Fill before shrink");
    test_fill_before_shrink().await;
    crate::klogln!(Info, "Test/FileCache", "Failed fill can retry");
    test_failed_fill_can_retry().await;
}
