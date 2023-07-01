//! Tool for (a)synchronous implementation.

/// Notifier for waiting [`Queue`](crate::Queue) operations.
pub trait Notify {
    /// Wake waiting dequeue operation.
    fn notify_dequeue(&self);
    /// Wake waiting enqueue operation.
    fn notify_enqueue(&self);
}

impl Notify for () {
    #[inline]
    fn notify_dequeue(&self) {}
    #[inline]
    fn notify_enqueue(&self) {}
}
