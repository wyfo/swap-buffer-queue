use std::task;

use crate::{
    loom::{
        hint,
        sync::atomic::{AtomicUsize, Ordering},
        thread, LoomUnsafeCell,
    },
    synchronized::waker::Waker,
};

// I have to reimplement AtomicWaker because it doesn't use standard `Waker`,
// see https://internals.rust-lang.org/t/thread-park-waker-a-waker-calling-thread-unpark/19114
pub(super) struct AtomicWaker {
    state: AtomicUsize,
    waker: LoomUnsafeCell<Option<Waker>>,
}

const WAITING: usize = 0;
const REGISTERING: usize = 0b01;
const WAKING: usize = 0b10;

impl AtomicWaker {
    pub fn new() -> Self {
        Self {
            state: AtomicUsize::new(WAITING),
            waker: LoomUnsafeCell::new(None),
        }
    }

    #[inline]
    pub(super) fn register(&self, cx: Option<&task::Context>) {
        match self
            .state
            .compare_exchange(WAITING, REGISTERING, Ordering::Acquire, Ordering::Acquire)
            .unwrap_or_else(|x| x)
        {
            WAITING => {
                // SAFETY: see `futures::task::AtomicWaker` implementation
                unsafe {
                    self.waker.with_mut(|w| match &mut *w {
                        Some(old_waker) if old_waker.will_wake(cx) => (),
                        _ => *w = Some(Waker::new(cx)),
                    });
                    let res = self.state.compare_exchange(
                        REGISTERING,
                        WAITING,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );

                    match res {
                        Ok(_) => {}
                        Err(actual) => {
                            debug_assert_eq!(actual, REGISTERING | WAKING);
                            let waker = self.waker.with_mut(|w| (*w).take()).unwrap();
                            self.state.swap(WAITING, Ordering::AcqRel);
                            waker.wake();
                        }
                    }
                }
            }
            WAKING => {
                match cx {
                    Some(cx) => cx.waker().wake_by_ref(),
                    None => thread::current().unpark(),
                }
                // SAFETY: see `AtomicWaker` implementation from tokio (it's needed by loom)
                hint::spin_loop();
            }
            state => {
                debug_assert!(state == REGISTERING || state == REGISTERING | WAKING);
            }
        }
    }

    #[inline]
    pub(super) fn wake(&self) {
        match self.state.fetch_or(WAKING, Ordering::AcqRel) {
            WAITING => {
                // SAFETY: see `futures::task::AtomicWaker` implementation
                let waker = unsafe { self.waker.with_mut(|w| (*w).take()) };
                self.state.fetch_and(!WAKING, Ordering::Release);
                if let Some(waker) = waker {
                    waker.wake()
                }
            }
            state => {
                debug_assert!(
                    state == REGISTERING || state == REGISTERING | WAKING || state == WAKING
                );
            }
        }
    }
}

impl Default for AtomicWaker {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: see `futures::task::AtomicWaker` implementation
unsafe impl Send for AtomicWaker {}

// SAFETY: see `futures::task::AtomicWaker` implementation
unsafe impl Sync for AtomicWaker {}

#[cfg(all(test, loom))]
mod tests {
    use std::{
        future::poll_fn,
        sync::Arc,
        task::Poll::{Pending, Ready},
    };

    use loom::{
        future::block_on,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    use super::AtomicWaker;

    struct Chan {
        num: AtomicUsize,
        task: AtomicWaker,
    }
    #[test]
    fn basic_notification() {
        const NUM_NOTIFY: usize = 2;

        loom::model(|| {
            let chan = Arc::new(Chan {
                num: AtomicUsize::new(0),
                task: AtomicWaker::default(),
            });

            for _ in 0..NUM_NOTIFY {
                let chan = chan.clone();

                thread::spawn(move || {
                    chan.num.fetch_add(1, Ordering::Relaxed);
                    chan.task.wake();
                });
            }

            block_on(poll_fn(move |cx| {
                chan.task.register(Some(cx));

                if NUM_NOTIFY == chan.num.load(Ordering::Relaxed) {
                    return Ready(());
                }

                Pending
            }));
        });
    }
}
