use std::task;

use crate::loom::thread;

#[derive(Debug)]
pub(super) enum Waker {
    Async(task::Waker),
    Sync(thread::Thread),
}

impl Waker {
    pub(super) fn new(cx: Option<&task::Context>) -> Self {
        match cx {
            Some(cx) => Self::Async(cx.waker().clone()),
            None => Self::Sync(thread::current()),
        }
    }

    pub(super) fn wake(self) {
        match self {
            Self::Async(waker) => waker.wake(),
            Self::Sync(thread) => thread.unpark(),
        }
    }
}
