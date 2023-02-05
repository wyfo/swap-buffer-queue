//! Tool for (a)synchronous implementation.

/// Notifier for waiting [`SBQueue`](crate::SBQueue) operations.
pub trait Notify {
    /// Wake waiting dequeue operation.
    ///
    /// Because of pending state (see [`SBQueue::try_dequeue`](crate::SBQueue::try_dequeue)),
    /// in case of concurrent enqueuing, the queue may not be ready to dequeue when this method is
    /// called, as indicated by `may_be_ready` parameter.
    ///
    /// However, trying to dequeue eagerly stop the buffer filling, waiting only for already begun
    /// enqueuing to end. On the other hand, waiting the end of enqueuing to dequeue will maximize
    /// the buffer capacity filled.
    fn notify_dequeue(&self, may_be_ready: bool);
    /// Wake waiting enqueue operation.
    fn notify_enqueue(&self);
}

impl Notify for () {
    fn notify_dequeue(&self, _may_be_ready: bool) {}
    fn notify_enqueue(&self) {}
}
