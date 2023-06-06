#[cfg(not(all(loom, test)))]
pub(crate) use std::sync::atomic;
#[cfg(not(all(loom, test)))]
#[cfg(feature = "std")]
pub(crate) use std::sync::{Condvar, Mutex};
#[rustfmt::skip]
#[cfg(not(all(loom, test)))]
pub use crossbeam_utils::Backoff;

#[cfg(all(loom, test))]
pub(crate) use loom::sync::atomic;
#[cfg(all(loom, test))]
#[cfg(feature = "std")]
pub(crate) use loom::sync::{Condvar, Mutex};
#[cfg(all(loom, test))]
struct Backoff;
#[cfg(all(loom, test))]
impl Backoff {
    fn new() -> Self {
        Self
    }
    fn snooze(&self) {}
    fn is_completed(&self) -> bool {
        true
    }
}
