use std::task;

use crossbeam_utils::CachePadded;

use crate::{
    loom::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    synchronized::waker::Waker,
};

#[derive(Debug, Default)]
pub(super) struct WakerList {
    wakers: Mutex<Vec<Waker>>,
    non_empty: CachePadded<AtomicBool>,
}

impl WakerList {
    pub(super) fn register(&self, cx: Option<&task::Context>) {
        let waker = Waker::new(cx);
        let mut wakers = self.wakers.lock().unwrap();
        if wakers.is_empty() {
            self.non_empty.store(true, Ordering::SeqCst);
        }
        wakers.push(waker);
    }

    #[inline]
    pub(super) fn wake(&self) {
        if self.non_empty.load(Ordering::Relaxed)
            && self
                .non_empty
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            self.wake_all();
        }
    }

    // not inlined
    fn wake_all(&self) {
        for waker in self.wakers.lock().unwrap().drain(..) {
            match waker {
                Waker::Async(waker) => waker.wake(),
                Waker::Sync(thread) => thread.unpark(),
            }
        }
    }
}
