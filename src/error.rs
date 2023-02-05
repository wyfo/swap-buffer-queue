//! Queue error types.

use std::fmt;

/// Error returned by [`SBQueue::try_enqueue`](crate::SBQueue::try_enqueue).
///
/// The value whose enqueuing has failed is embedded within the error.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TryEnqueueError<T> {
    /// The queue doesn't have sufficient capacity to enqueue the give value.
    InsufficientCapacity(T),
    /// The queue is closed.
    Closed(T),
}

impl<T> TryEnqueueError<T> {
    /// Returns the value whose enqueuing has failed
    pub fn inner(self) -> T {
        match self {
            Self::InsufficientCapacity(v) | Self::Closed(v) => v,
        }
    }
}

/// Error returned by [`SBQueue::enqueue`](crate::SBQueue::enqueue).
///
/// It means the queue is closed.
///
/// The value whose enqueuing has failed is embedded within the error.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct EnqueueError<T>(pub T);

impl<T> EnqueueError<T> {
    /// Returns the value whose enqueuing has failed
    pub fn inner(self) -> T {
        self.0
    }
}

/// Error returned by [`SBQueue::try_dequeue`](crate::SBQueue::try_dequeue).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TryDequeueError {
    /// The queue is empty.
    Empty,
    /// There is a concurrent enqueuing that need to end before dequeuing.
    Pending,
    /// The queue is closed.
    Closed,
    /// The queue is concurrently dequeued.
    ///
    /// - A previous [`BufferSlice`](crate::buffer::BufferSlice`) may not have been dropped;
    /// - The queue is dequeued in another thread.
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

/// Error returned by [`SBQueue::dequeue`](crate::SBQueue::dequeue).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DequeueError {
    /// The queue is closed.
    Closed,
    /// The queue is concurrently dequeued.
    ///
    /// - A previous [`BufferSlice`](crate::buffer::BufferSlice`) may not have been dropped;
    /// - The queue is dequeued in another thread.
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
