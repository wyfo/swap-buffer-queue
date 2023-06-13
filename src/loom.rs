#[cfg(not(all(loom, test)))]
pub(crate) use std::sync::atomic;
#[cfg(not(all(loom, test)))]
#[cfg(feature = "std")]
pub(crate) use std::sync::Mutex;
#[rustfmt::skip]
#[cfg(not(all(loom, test)))]
pub(crate) use crossbeam_utils::Backoff;

#[cfg(all(loom, test))]
pub(crate) use loom::sync::atomic;
#[cfg(all(loom, test))]
#[cfg(feature = "std")]
pub(crate) use loom::sync::Mutex;
#[cfg(all(loom, test))]
pub(crate) struct Backoff;
#[cfg(all(loom, test))]
impl Backoff {
    pub(crate) fn new() -> Self {
        Self
    }
    pub(crate) fn snooze(&self) {
        std::thread::yield_now()
    }
    pub(crate) fn spin(&self) {
        std::hint::spin_loop()
    }
    pub(crate) fn is_completed(&self) -> bool {
        true
    }
}
