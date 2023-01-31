//! Tool for (a)synchronous implementation.

/// Notifier for waiting [`SBQueue`](crate::SBQueue) operations.
pub trait Notify {
    /// Wake waiting dequeue operation.
    fn notify_dequeue(&self);
    /// Wake waiting enqueue operation.
    fn notify_enqueue(&self);
}

impl Notify for () {
    fn notify_dequeue(&self) {}
    fn notify_enqueue(&self) {}
}
