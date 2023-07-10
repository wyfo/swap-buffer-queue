use std::{cell::Cell, mem::MaybeUninit, task, thread};

use crate::{
    loom::atomic::{AtomicU8, Ordering},
    synchronized::Waker,
};

const EMPTY: u8 = 0b001;
const REGISTERING: u8 = 0b001;
const REGISTERED: u8 = 0b010;
const WAKING_FLAG: u8 = 0b100;

pub(crate) struct AtomicWaker {
    waker: Cell<MaybeUninit<Waker>>,
    state: AtomicU8,
}

impl Default for AtomicWaker {
    fn default() -> Self {
        Self {
            waker: Cell::new(MaybeUninit::uninit()),
            state: AtomicU8::new(0),
        }
    }
}

// SAFETY: `AtomicWaker` waker `Cell` access is synchronized using state
// (see `AtomicWaker::register`/`AtomicWaker::wake`)
unsafe impl Send for AtomicWaker {}

// SAFETY: `AtomicWaker` waker `Cell` access is synchronized using state
// (see `AtomicWaker::register`/`AtomicWaker::wake`)
unsafe impl Sync for AtomicWaker {}

impl AtomicWaker {
    #[inline]
    pub(crate) fn register(&self, cx: Option<&task::Context>) {
        let mut state = self.state.load(Ordering::Relaxed);
        loop {
            if state != EMPTY && state != REGISTERED {
                match cx {
                    Some(cx) => cx.waker().wake_by_ref(),
                    None => thread::current().unpark(),
                }
                return;
            }
            match self.state.compare_exchange_weak(
                state,
                REGISTERING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(s) => state = s,
            }
        }
        let prev = self.waker.replace(MaybeUninit::new(Waker::new(cx)));
        if state == REGISTERED {
            // SAFETY: state was previously `REGISTERED`, so waker was initialized.
            unsafe { prev.assume_init() };
        }
        if let Err(state) = self.state.compare_exchange(
            REGISTERING,
            REGISTERED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            debug_assert_eq!(state, REGISTERING | WAKING_FLAG);
            // SAFETY: waker has been initialized a few lines above, and cannot be read
            // by `Self::wake` because state has not been set to `REGISTERED`.
            unsafe { self.waker.replace(MaybeUninit::uninit()).assume_init() }.wake();
            self.state.store(EMPTY, Ordering::Release);
        }
    }

    #[inline]
    pub(crate) fn wake(&self) {
        let mut state = self.state.load(Ordering::Relaxed);
        loop {
            if state != REGISTERING && state != REGISTERED {
                return;
            }
            match self.state.compare_exchange_weak(
                state,
                state | WAKING_FLAG,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(REGISTERED) => break,
                Ok(_) => return,
                Err(s) => state = s,
            }
        }
        // SAFETY: state was `REGISTERED`, so waker has been registered and can no more
        // be read/modified by `Self::register`
        unsafe { self.waker.replace(MaybeUninit::uninit()).assume_init() }.wake();
        self.state.store(EMPTY, Ordering::Release);
    }
}
