use std::task;

use crate::loom::thread;

#[derive(Debug)]
pub(super) enum Waker {
    Async(task::Waker),
    Sync(thread::Thread),
}

impl Waker {
    #[inline]
    pub(super) fn new(cx: Option<&task::Context>) -> Self {
        match cx {
            Some(cx) => Self::Async(cx.waker().clone()),
            None => Self::Sync(thread::current()),
        }
    }

    #[inline]
    pub(super) fn will_wake(&self, cx: Option<&task::Context>) -> bool {
        match (self, cx) {
            (Self::Async(w), Some(cx)) => w.will_wake(cx.waker()),
            _ => false,
        }
    }

    #[inline]
    pub(super) fn wake(self) {
        match self {
            Self::Async(waker) => waker.wake(),
            Self::Sync(thread) => thread.unpark(),
        }
    }
}
