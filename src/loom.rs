#[cfg(not(all(loom, test)))]
pub use std::sync::atomic;
#[cfg(not(all(loom, test)))]
#[cfg(all(feature = "std", not(feature = "parking_lot")))]
pub(crate) use std::sync::{Condvar, Mutex};
#[rustfmt::skip]
#[cfg(not(all(loom, test)))]
#[cfg(feature = "parking_lot")]
pub use self::parking_lot::{Condvar, Mutex};

#[cfg(feature = "parking_lot")]
mod parking_lot {
    type LockResult<T> = Result<T, std::convert::Infallible>;

    #[derive(Debug, Default)]
    pub struct Condvar(parking_lot::Condvar);

    impl Condvar {
        pub fn notify_all(&self) {
            self.0.notify_all();
        }

        pub fn wait<'a, T>(
            &self,
            mut guard: parking_lot::MutexGuard<'a, T>,
        ) -> LockResult<parking_lot::MutexGuard<'a, T>> {
            self.0.wait(&mut guard);
            Ok(guard)
        }

        pub fn wait_timeout<'a, T>(
            &self,
            mut guard: parking_lot::MutexGuard<'a, T>,
            dur: std::time::Duration,
        ) -> LockResult<(
            parking_lot::MutexGuard<'a, T>,
            parking_lot::WaitTimeoutResult,
        )> {
            let res = self.0.wait_for(&mut guard, dur);
            Ok((guard, res))
        }
    }

    #[derive(Debug, Default)]
    pub struct Mutex<T>(parking_lot::Mutex<T>);

    impl<T> Mutex<T> {
        pub fn lock(&self) -> LockResult<parking_lot::MutexGuard<'_, T>> {
            Ok(self.0.lock())
        }
    }
}

#[cfg(all(loom, test))]
pub(crate) use loom::sync::atomic;
#[cfg(all(loom, test))]
#[cfg(feature = "std")]
pub(crate) use loom::sync::{Condvar, Mutex};
