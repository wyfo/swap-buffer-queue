#![deny(clippy::dbg_macro)]
#![deny(clippy::semicolon_if_nothing_returned)]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
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
//! # Examples
//!
//! ```rust
//! # use std::ops::Deref;
//! # use swap_buffer_queue::{buffer::VecBuffer, Queue};
//! // Initialize the queue with a capacity
//! let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
//! // Enqueue some values
//! queue.try_enqueue(0).unwrap();
//! queue.try_enqueue(1).unwrap();
//! // Dequeue a slice to the enqueued values
//! let slice = queue.try_dequeue().unwrap();
//! assert_eq!(slice.deref(), &[0, 1]);
//! // Enqueued values can also be retrieved
//! assert_eq!(slice.into_iter().collect::<Vec<_>>(), vec![0, 1]);
//! ```

extern crate alloc;
#[cfg(not(feature = "std"))]
extern crate core as std;

pub mod buffer;
pub mod error;
mod loom;
pub mod notify;
mod queue;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
mod synchronized;
mod utils;
#[cfg(feature = "write")]
#[cfg_attr(docsrs, doc(cfg(feature = "write")))]
pub mod write;
#[cfg(feature = "write")]
#[cfg_attr(docsrs, doc(cfg(feature = "write")))]
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod write_vectored;

pub use queue::Queue;
#[cfg(feature = "std")]
pub use synchronized::SynchronizedNotifier;
#[cfg(feature = "std")]
/// [`Queue`] with [`SynchronizedNotifier`]
pub type SynchronizedQueue<B> = Queue<B, synchronized::SynchronizedNotifier>;
