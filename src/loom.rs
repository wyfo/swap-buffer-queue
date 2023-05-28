#![allow(unused_imports)]
/// Do not export UnsafeCell because it's used for a shared slice, so not compatible with loom model

#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(loom))]
#[cfg(feature = "sync")]
pub(crate) use std::sync::{Condvar, Mutex, MutexGuard};

#[cfg(loom)]
pub(crate) use loom::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Condvar, Mutex, MutexGuard,
};
