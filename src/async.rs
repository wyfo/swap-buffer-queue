//! Asynchronous implementation of [`SBQueue`].

use std::{
    future::poll_fn,
    task::{Poll, Waker},
};

use crossbeam_utils::CachePadded;
use futures::{stream, Stream, StreamExt};

use crate::{
    buffer::{Buffer, BufferSlice, BufferValue, Drain},
    error::{DequeueError, EnqueueError, TryDequeueError, TryEnqueueError},
    loom::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    notify::Notify,
    queue::SBQueue,
};

/// An asynchronous notifier.
#[derive(Debug, Default)]
pub struct AsyncNotifier<const EAGER: bool = false> {
    wakers: Mutex<Vec<Waker>>,
    dequeue_waiting: CachePadded<AtomicBool>,
    enqueue_waiting: CachePadded<AtomicBool>,
}

impl<const EAGER: bool> AsyncNotifier<EAGER> {
    async fn wait_until(&self, waiting: &AtomicBool) {
        poll_fn(|cx| {
            let mut wakers = self.wakers.lock().unwrap();
            if !waiting.load(Ordering::SeqCst) {
                return Poll::Ready(());
            }
            wakers.push(cx.waker().clone());
            Poll::Pending
        })
        .await;
    }

    fn notify(&self, waiting: &AtomicBool) {
        if waiting.load(Ordering::Relaxed)
            && waiting
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
        {
            for waker in self.wakers.lock().unwrap().drain(..) {
                waker.wake();
            }
        }
    }
}

impl<const EAGER: bool> Notify for AsyncNotifier<EAGER> {
    fn notify_dequeue(&self, may_be_ready: bool) {
        if EAGER || may_be_ready {
            self.notify(&self.dequeue_waiting);
        }
    }

    fn notify_enqueue(&self) {
        self.notify(&self.enqueue_waiting);
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
            match self.try_enqueue(value) {
                Err(TryEnqueueError::InsufficientCapacity(v)) if v.size() <= self.capacity() => {
                    value = v;
                }
                res => return res,
            };
            self.notify().enqueue_waiting.store(true, Ordering::SeqCst);
            match self.try_enqueue(value) {
                Err(TryEnqueueError::InsufficientCapacity(v)) if v.size() <= self.capacity() => {
                    value = v;
                }
                res => return res,
            };
            self.notify()
                .wait_until(&self.notify().enqueue_waiting)
                .await;
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
        loop {
            match self.try_dequeue() {
                Ok(buf) => return Ok(buf),
                Err(TryDequeueError::Closed) => return Err(DequeueError::Closed),
                Err(TryDequeueError::Conflict) => return Err(DequeueError::Conflict),
                Err(TryDequeueError::Empty | TryDequeueError::Pending) => {}
            }
            self.notify().dequeue_waiting.store(true, Ordering::SeqCst);
            match self.try_dequeue() {
                Ok(buf) => return Ok(buf),
                Err(TryDequeueError::Closed) => return Err(DequeueError::Closed),
                Err(TryDequeueError::Conflict) => return Err(DequeueError::Conflict),
                Err(TryDequeueError::Empty | TryDequeueError::Pending) => {}
            }
            self.notify()
                .wait_until(&self.notify().dequeue_waiting)
                .await;
        }
    }
}

impl<B, const EAGER: bool> SBQueue<B, AsyncNotifier<EAGER>>
where
    B: Buffer + Drain,
{
    /// Returns an stream over the element of the queue (see [`BufferIter`]).
    ///
    /// # Examples
    /// ```
    /// # use futures::StreamExt;
    /// # use swap_buffer_queue::AsyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # tokio_test::block_on(async {
    /// let queue: AsyncSBQueue<VecBuffer<usize>> = AsyncSBQueue::with_capacity(42);
    /// queue.try_enqueue(0).unwrap();
    /// queue.try_enqueue(1).unwrap();
    ///
    /// let mut stream = Box::pin(queue.stream());
    /// assert_eq!(stream.next().await, Some(0));
    /// drop(stream);
    /// let mut stream = Box::pin(queue.stream());
    /// assert_eq!(stream.next().await, Some(1));
    /// queue.close(); // close in order to stop the stream
    /// assert_eq!(stream.next().await, None);
    /// # })
    /// ```
    pub fn stream(&self) -> impl Stream<Item = B::Value> + '_ {
        stream::repeat_with(|| stream::once(self.dequeue()))
            .flatten()
            .take_while(|res| {
                let is_ok = res.is_ok();
                async move { is_ok }
            })
            .flat_map(|res| stream::iter(res.unwrap()))
    }
}
