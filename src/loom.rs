#[cfg(not(all(loom, test)))]
pub(crate) use std::sync::atomic;
#[cfg(not(all(loom, test)))]
#[cfg(feature = "std")]
pub(crate) use std::sync::Mutex;
#[cfg(not(all(loom, test)))]
pub(crate) const SPIN_LIMIT: usize = 64;
#[cfg(not(all(loom, test)))]
pub(crate) const BACKOFF_LIMIT: usize = 6;

#[cfg(all(loom, test))]
pub(crate) use loom::sync::atomic;
#[cfg(all(loom, test))]
#[cfg(feature = "std")]
pub(crate) use loom::sync::Mutex;
#[cfg(all(loom, test))]
pub(crate) const SPIN_LIMIT: usize = 1;
#[cfg(all(loom, test))]
pub(crate) const BACKOFF_LIMIT: usize = 1;
