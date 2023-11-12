use std::fmt;

use crate::{
    buffer::ValueIter,
    error::{DequeueError, TryDequeueError, TryEnqueueError},
};

pub(crate) trait Extract<T> {
    fn extract(self) -> T;
}

impl<T> Extract<T> for [T; 1] {
    fn extract(self) -> T {
        let [value] = self;
        value
    }
}

impl<T> Extract<T> for ValueIter<T> {
    fn extract(self) -> T {
        self.0
    }
}

pub(crate) fn try_send_error<T>(error: TryEnqueueError<impl Extract<T>>) -> TrySendError<T> {
    match error {
        TryEnqueueError::InsufficientCapacity(x) => TrySendError::InsufficientCapacity(x.extract()),
        TryEnqueueError::Closed(x) => TrySendError::Closed(x.extract()),
    }
}

pub(crate) use try_send_error as send_error;

pub(crate) fn try_recv_err(error: TryDequeueError) -> TryRecvError {
    match error {
        TryDequeueError::Empty => TryRecvError::Empty,
        TryDequeueError::Pending => TryRecvError::Pending,
        TryDequeueError::Closed => TryRecvError::Closed,
        TryDequeueError::Conflict => unreachable!(),
    }
}

pub(crate) fn recv_err(error: DequeueError) -> RecvError {
    match error {
        DequeueError::Closed => RecvError,
        _ => unreachable!(),
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum TrySendError<T> {
    InsufficientCapacity(T),
    Closed(T),
}

impl<T> TrySendError<T> {
    pub fn into_inner(self) -> T {
        match self {
            Self::InsufficientCapacity(v) | Self::Closed(v) => v,
        }
    }
}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientCapacity(_) => write!(f, "TrySendError::Full(_)"),
            Self::Closed(_) => write!(f, "TrySendError::Closed(_)"),
        }
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = match self {
            Self::InsufficientCapacity(_) => "channel is full",
            Self::Closed(_) => "channel is closed",
        };
        write!(f, "{error}")
    }
}

impl<T> std::error::Error for TrySendError<T> {}

pub type SendError<T> = TrySendError<T>;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TryRecvError {
    Empty,
    Pending,
    Closed,
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = match self {
            Self::Empty => "channel is empty",
            Self::Pending => "waiting for concurrent sending end",
            Self::Closed => "channel is closed",
        };
        write!(f, "{error}")
    }
}

impl std::error::Error for TryRecvError {}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct RecvError;

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "channel is closed")
    }
}

impl std::error::Error for RecvError {}
