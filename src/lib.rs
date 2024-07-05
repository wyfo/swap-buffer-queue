#![forbid(clippy::dbg_macro)]
#![forbid(clippy::semicolon_if_nothing_returned)]
#![forbid(missing_docs)]
#![forbid(unsafe_op_in_unsafe_fn)]
#![forbid(clippy::undocumented_unsafe_blocks)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
//! # swap-buffer-queue
//! A buffering MPSC queue.
//!
//! This library is intended to be a (better, I hope) alternative to traditional MPSC queues
//! in the context of a buffering consumer, by moving the buffering part directly into the queue.
//!
//! It is especially well suited for IO writing workflow, see [`mod@write`] and [`write_vectored`].
//!
//! The crate is *no_std* (some buffer implementations may require `std`).
//!
//! In addition to the low level `Queue` implementation, a higher level `SynchronizedQueue` is
//! provided with both blocking and asynchronous methods.
//!
//! # Examples
//!
//! ```rust
//! # use std::ops::Deref;
//! # use swap_buffer_queue::{buffer::{IntoValueIter, VecBuffer}, Queue};
//! // Initialize the queue with a capacity
//! let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
//! // Enqueue some value
//! queue.try_enqueue([0]).unwrap();
//! // Multiple values can be enqueued at the same time
//! // (optimized compared to multiple enqueuing)
//! queue.try_enqueue([1, 2]).unwrap();
//! let mut values = vec![3, 4];
//! queue
//!     .try_enqueue(values.drain(..).into_value_iter())
//!     .unwrap();
//! // Dequeue a slice to the enqueued values
//! let slice = queue.try_dequeue().unwrap();
//! assert_eq!(slice.deref(), &[0, 1, 2, 3, 4]);
//! // Enqueued values can also be retrieved
//! assert_eq!(slice.into_iter().collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
//! ```

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod buffer;
pub mod error;
mod loom;
pub mod notify;
mod queue;
#[cfg(feature = "std")]
mod synchronized;
mod utils;
#[cfg(feature = "write")]
pub mod write;
#[cfg(feature = "write")]
#[cfg(feature = "std")]
pub mod write_vectored;

pub use queue::Queue;
#[cfg(feature = "std")]
pub use synchronized::{SynchronizedNotifier, SynchronizedQueue};
