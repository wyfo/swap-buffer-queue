#[cfg(not(all(loom, test)))]
pub use std::sync::atomic;
#[cfg(not(all(loom, test)))]
#[cfg(feature = "std")]
pub(crate) use std::sync::{Condvar, Mutex};

#[cfg(all(loom, test))]
pub(crate) use loom::sync::atomic;
#[cfg(all(loom, test))]
#[cfg(feature = "std")]
pub(crate) use loom::sync::{Condvar, Mutex};
