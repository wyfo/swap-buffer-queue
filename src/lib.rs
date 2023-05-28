// #![warn(missing_debug_implementations)]
// #![deny(clippy::dbg_macro)]
// #![deny(clippy::map_unwrap_or)]
// #![deny(clippy::semicolon_if_nothing_returned)]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
//! # swap-buffer-queue
//! A buffering MPSC queue.
//!
//! This library is intended to be a (better, I hope) alternative to traditional MPSC queues
//! in the context of a buffering consumer, by moving the buffering part directly into the queue.
//!
//! It is especially well suited for IO writing workflow, see [`write`](crate::write) and
//! [`write_vectored`].
//!
//! The crate is *no_std* (some buffer implementations may require `std`).
//!
//! # Examples
//!
//! ```rust
//! # use std::ops::Deref;
//! # use swap_buffer_queue::{buffer::VecBuffer, SBQueue};
//! // Initialize the queue with a capacity
//! let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
//! // Enqueue some values
//! queue.try_enqueue(0).unwrap();
//! queue.try_enqueue(1).unwrap();
//! // Dequeue a slice to the enqueued values
//! let slice = queue.try_dequeue().unwrap();
//! assert_eq!(slice.deref(), &[0, 1]);
//! // Enqueued values can also be retrieved
//! assert_eq!(slice.into_iter().collect::<Vec<_>>(), vec![0, 1]);
//! ```

#[cfg(not(feature = "std"))]
extern crate core as std;

// Define this module before async and sync ones, in order to order the documentation
mod queue;

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub mod r#async;
pub mod buffer;
pub mod error;
mod loom;
pub mod notify;
#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
pub mod sync;
#[cfg(any(feature = "write", feature = "write-vectored"))]
mod utils;
#[cfg(feature = "write")]
#[cfg_attr(docsrs, doc(cfg(feature = "write")))]
pub mod write;
#[cfg(feature = "write-vectored")]
#[cfg_attr(docsrs, doc(cfg(feature = "write-vectored")))]
pub mod write_vectored;

pub use queue::SBQueue;
#[cfg(feature = "async")]
/// An asynchronous implementation of [`SBQueue`].
pub type AsyncSBQueue<B, const EAGER: bool = false> = SBQueue<B, r#async::AsyncNotifier<EAGER>>;
#[cfg(feature = "sync")]
/// A synchronous implementation of [`SBQueue`].
pub type SyncSBQueue<B, const EAGER: bool = false> = SBQueue<B, sync::SyncNotifier<EAGER>>;
