//! Asynchronous implementation of [`SBQueue`].

use std::{future, task::Poll};

use futures::task::AtomicWaker;

use crate::{
    buffer::{Buffer, BufferSlice, BufferValue},
    error::{DequeueError, EnqueueError, TryDequeueError, TryEnqueueError},
    notify::Notify,
    queue::SBQueue,
};

/// An asynchronous notifier.
#[derive(Debug, Default)]
pub struct AsyncNotifier<const EAGER: bool = false> {
    waker: AtomicWaker,
    notify: tokio::sync::Notify,
}

impl<const EAGER: bool> Notify for AsyncNotifier<EAGER> {
    fn notify_dequeue(&self, may_be_ready: bool) {
        if EAGER || may_be_ready {
            self.waker.wake();
        }
    }

    fn notify_enqueue(&self) {
        self.notify.notify_waiters();
    }
}

impl<B, const EAGER: bool> SBQueue<B, AsyncNotifier<EAGER>>
where
    B: Buffer,
{
    /// Enqueues the given value inside the queue.
    ///
    /// This method extends [`try_enqueue`](SBQueue::try_enqueue) by waiting asynchronously
    /// [`AsyncNotifier::notify_enqueue`] call, i.e. when a buffer is dequeued, in case of
    /// insufficient capacity.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::AsyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{EnqueueError, TryEnqueueError};
    /// # tokio_test::block_on(async {
    /// let queue: Arc<AsyncSBQueue<VecBuffer<usize>>> = Arc::new(AsyncSBQueue::with_capacity(1));
    /// queue.try_enqueue(0).unwrap();
    /// assert_eq!(
    ///     queue.try_enqueue(0),
    ///     Err(TryEnqueueError::InsufficientCapacity(0))
    /// );
    /// // queue is full, let's spawn an enqueuing task and dequeue
    /// let queue_clone = queue.clone();
    /// let task = tokio::spawn(async move { queue_clone.enqueue(1).await });
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[0]);
    /// // enqueuing task has succeeded
    /// task.await.unwrap().unwrap();
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[1]);
    /// // let's close the queue
    /// queue.try_enqueue(2).unwrap();
    /// let queue_clone = queue.clone();
    /// let task = tokio::spawn(async move { queue_clone.enqueue(3).await });
    /// queue.close();
    /// assert_eq!(task.await.unwrap(), Err(EnqueueError::Closed(3)));
    /// # })
    /// ```
    pub async fn enqueue<T>(&self, mut value: T) -> Result<(), EnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        loop {
            let notified = self.notify().notify.notified();
            match self.try_enqueue(value) {
                Err(TryEnqueueError::InsufficientCapacity(v)) if v.size() <= self.capacity() => {
                    value = v
                }
                res => return res,
            };
            notified.await;
        }
    }

    /// Dequeues a buffer with all enqueued values from the queue.
    ///
    /// This method extends [`try_dequeue`](SBQueue::try_dequeue) by waiting asynchronously
    /// [`AsyncNotifier::notify_dequeue`] call, i.e. when a value is enqueued, in case of
    /// empty queue.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::AsyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{DequeueError, TryDequeueError};
    /// # tokio_test::block_on(async {
    /// let queue: Arc<AsyncSBQueue<VecBuffer<usize>>> = Arc::new(AsyncSBQueue::with_capacity(1));
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Empty);
    /// // queue is empty, let's spawn a dequeuing task and enqueue
    /// let queue_clone = queue.clone();
    /// let task = tokio::spawn(async move {
    ///     Ok::<_, DequeueError>(queue_clone.dequeue().await?.into_iter().collect::<Vec<_>>())
    /// });
    /// queue.try_enqueue(0).unwrap();
    /// // dequeuing task has succeeded
    /// assert_eq!(task.await.unwrap().unwrap().deref(), &[0]);
    /// // let's close the queue
    /// let queue_clone = queue.clone();
    /// let task = tokio::spawn(async move {
    ///     Ok::<_, DequeueError>(queue_clone.dequeue().await?.into_iter().collect::<Vec<_>>())
    /// });
    /// queue.close();
    /// assert_eq!(task.await.unwrap().unwrap_err(), DequeueError::Closed);
    /// # })
    /// ```
    pub async fn dequeue(&self) -> Result<BufferSlice<B, AsyncNotifier<EAGER>>, DequeueError> {
        future::poll_fn(|cx| {
            match self.try_dequeue() {
                Ok(buf) => return Poll::Ready(Ok(buf)),
                Err(TryDequeueError::Empty | TryDequeueError::Pending) => {}
                Err(TryDequeueError::Closed) => return Poll::Ready(Err(DequeueError::Closed)),
                Err(TryDequeueError::Conflict) => return Poll::Ready(Err(DequeueError::Conflict)),
            }
            self.notify().waker.register(cx.waker());
            match self.try_dequeue() {
                Ok(buf) => Poll::Ready(Ok(buf)),
                Err(TryDequeueError::Empty | TryDequeueError::Pending) => Poll::Pending,
                Err(TryDequeueError::Closed) => Poll::Ready(Err(DequeueError::Closed)),
                Err(TryDequeueError::Conflict) => Poll::Ready(Err(DequeueError::Conflict)),
            }
        })
        .await
    }
}
