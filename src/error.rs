//! Queue error types.

use std::fmt;

/// Error returned by [`Queue::try_enqueue`](crate::Queue::try_enqueue).
///
/// The value whose enqueuing has failed is embedded within the error.
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum TryEnqueueError<T> {
    /// The queue doesn't have sufficient capacity to enqueue the give value.
    InsufficientCapacity(T),
    /// The queue is closed.
    Closed(T),
}

impl<T> TryEnqueueError<T> {
    /// Returns the value whose enqueuing has failed
    pub fn into_inner(self) -> T {
        match self {
            Self::InsufficientCapacity(v) | Self::Closed(v) => v,
        }
    }
}

impl<T> fmt::Debug for TryEnqueueError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientCapacity(_) => write!(f, "TryEnqueueError::InsufficientCapacity(_)"),
            Self::Closed(_) => write!(f, "TryEnqueueError::Closed(_)"),
        }
    }
}

impl<T> fmt::Display for TryEnqueueError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = match self {
            Self::InsufficientCapacity(_) => "queue has insufficient capacity",
            Self::Closed(_) => "queue is closed",
        };
        write!(f, "{error}")
    }
}

#[cfg(feature = "std")]
impl<T> std::error::Error for TryEnqueueError<T> {}

/// Error returned by [`SynchronizedQueue::enqueue`](crate::SynchronizedQueue::enqueue)/[`SynchronizedQueue::enqueue_async`](crate::SynchronizedQueue::enqueue_async)
pub type EnqueueError<T> = TryEnqueueError<T>;

/// Error returned by [`Queue::try_dequeue`](crate::Queue::try_dequeue).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TryDequeueError {
    /// The queue is empty.
    Empty,
    /// There is a concurrent enqueuing that need to end before dequeuing.
    Pending,
    /// The queue is closed.
    Closed,
    /// The queue is concurrently dequeued.
    Conflict,
}

impl fmt::Display for TryDequeueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = match self {
            Self::Empty => "queue is empty",
            Self::Pending => "waiting for concurrent enqueuing end",
            Self::Closed => "queue is closed",
            Self::Conflict => "queue is concurrently dequeued",
        };
        write!(f, "{error}")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TryDequeueError {}

/// Error returned by [`SynchronizedQueue::dequeue`](crate::SynchronizedQueue::dequeue)/
/// [`SynchronizedQueue::dequeue_async`](crate::SynchronizedQueue::dequeue_async).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DequeueError {
    /// The queue is closed.
    Closed,
    /// The queue is concurrently dequeued.
    Conflict,
}

impl fmt::Display for DequeueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = match self {
            Self::Closed => "queue is closed",
            Self::Conflict => "queue is concurrently dequeued",
        };
        write!(f, "{error}")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DequeueError {}
