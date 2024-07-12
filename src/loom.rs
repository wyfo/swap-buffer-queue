#[cfg(not(all(loom, test)))]
mod without_loom {
    #[cfg(not(feature = "std"))]
    pub(crate) use core::sync;
    pub(crate) use core::{cell, hint};
    #[cfg(feature = "std")]
    pub(crate) use std::{sync, thread};

    pub(crate) const SPIN_LIMIT: usize = 64;
    pub(crate) const BACKOFF_LIMIT: usize = 6;

    #[derive(Debug, Default)]
    pub(crate) struct LoomUnsafeCell<T: ?Sized>(cell::UnsafeCell<T>);

    impl<T: ?Sized> LoomUnsafeCell<T> {
        pub(crate) fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
            f(self.0.get())
        }

        pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
            f(self.0.get())
        }
    }

    #[cfg(feature = "std")]
    impl<T> LoomUnsafeCell<T> {
        pub(crate) fn new(data: T) -> Self {
            Self(cell::UnsafeCell::new(data))
        }
    }
}

#[cfg(not(all(loom, test)))]
pub(crate) use without_loom::*;

#[cfg(all(loom, test))]
mod with_loom {
    #[cfg(feature = "std")]
    pub(crate) use loom::thread;
    pub(crate) use loom::{cell, hint, sync};

    pub(crate) const SPIN_LIMIT: usize = 1;
    pub(crate) const BACKOFF_LIMIT: usize = 1;
    pub(crate) use cell::UnsafeCell as LoomUnsafeCell;
}

#[cfg(all(loom, test))]
pub(crate) use with_loom::*;
