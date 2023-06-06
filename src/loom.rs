#![allow(unused_imports)]
use std::sync::atomic;
/// Do not export UnsafeCell because it's used for a shared slice, so not compatible with loom model

#[cfg(not(all(loom, test)))]
pub(crate) use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(all(loom, test)))]
#[cfg(feature = "std")]
pub(crate) use std::sync::{atomic::AtomicBool, Condvar, Mutex, MutexGuard};

#[cfg(all(loom, test))]
pub(crate) use loom::sync::atomic::{AtomicUsize, Ordering};
#[cfg(all(loom, test))]
#[cfg(feature = "sync")]
pub(crate) use loom::sync::{atomic::AtomicBool, Condvar, Mutex, MutexGuard};
