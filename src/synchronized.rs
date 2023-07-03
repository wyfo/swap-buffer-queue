use std::{
    future::poll_fn,
    iter,
    task::{Context, Poll, Waker},
    thread,
    thread::Thread,
    time::{Duration, Instant},
};

use crossbeam_utils::CachePadded;

use crate::{
    buffer::{Buffer, BufferSlice, BufferValue, Drain},
    error::{DequeueError, EnqueueError, TryDequeueError, TryEnqueueError},
    loom::{
        atomic::{AtomicBool, Ordering},
        Mutex, SPIN_LIMIT,
    },
    notify::Notify,
    Queue,
};

#[derive(Debug)]
enum Waiter {
    Async(Waker),
    Sync(Thread),
}

#[derive(Debug, Default)]
struct WaiterList {
    waiters: Mutex<Vec<Waiter>>,
    non_empty: CachePadded<AtomicBool>,
}

impl WaiterList {
    fn register(&self, cx: Option<&Context>) {
        let mut waiters = self.waiters.lock().unwrap();
        if waiters.is_empty() {
            self.non_empty.store(true, Ordering::SeqCst);
        }
        waiters.push(match cx {
            Some(cx) => Waiter::Async(cx.waker().clone()),
            None => Waiter::Sync(thread::current()),
        });
    }

    fn wait_until(&self, deadline: Option<Instant>) -> bool {
        match deadline.map(|d| d.checked_duration_since(Instant::now())) {
            Some(Some(timeout)) => thread::park_timeout(timeout),
            Some(None) => return false,
            None => thread::park(),
        }
        true
    }

    #[inline]
    fn should_wake(&self) -> bool {
        self.non_empty.load(Ordering::Relaxed)
            && self
                .non_empty
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
    }

    fn wake_all(&self) {
        for waiter in self.waiters.lock().unwrap().drain(..) {
            match waiter {
                Waiter::Async(waker) => waker.wake(),
                Waiter::Sync(thread) => thread.unpark(),
            }
        }
    }
}

/// Synchronized (a)synchronous [`Notify`] implementation.
#[derive(Debug, Default)]
pub struct SynchronizedNotifier {
    enqueuers: WaiterList,
    dequeuers: WaiterList,
}

impl Notify for SynchronizedNotifier {
    #[inline]
    fn notify_dequeue(&self) {
        if self.dequeuers.should_wake() {
            self.dequeuers.wake_all();
        }
    }

    #[inline]
    fn notify_enqueue(&self) {
        if self.enqueuers.should_wake() {
            self.enqueuers.wake_all();
        }
    }
}

impl<B> Queue<B, SynchronizedNotifier>
where
    B: Buffer,
{
    fn enqueue_sync<T>(
        &self,
        mut value: T,
        deadline: Option<Instant>,
    ) -> Result<(), TryEnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        loop {
            match try_enqueue(self, value, None) {
                Ok(res) => return res,
                Err(v) => value = v,
            };
            if !self.notify().enqueuers.wait_until(deadline) {
                return self.try_enqueue(value);
            }
        }
    }

    /// Enqueues the given value inside the queue.
    ///
    /// This method extends [`try_enqueue`](Queue::try_enqueue) by waiting synchronously
    /// [`SynchronizedNotifier::notify_enqueue`] call, i.e. when a buffer is dequeued, in case of
    /// insufficient capacity.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use swap_buffer_queue::SynchronizedQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{EnqueueError, TryEnqueueError};
    /// let queue: Arc<SynchronizedQueue<VecBuffer<usize>>> =
    ///     Arc::new(SynchronizedQueue::with_capacity(1));
    /// queue.try_enqueue(0).unwrap();
    /// assert_eq!(
    ///     queue.try_enqueue(1),
    ///     Err(TryEnqueueError::InsufficientCapacity(1))
    /// );
    /// // queue is full, let's spawn an enqueuing task and dequeue
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || queue_clone.enqueue(1));
    /// std::thread::sleep(Duration::from_millis(1));
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[0]);
    /// // enqueuing task has succeeded
    /// task.join().unwrap().unwrap();
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[1]);
    /// // let's close the queue
    /// queue.try_enqueue(2).unwrap();
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || queue_clone.enqueue(3));
    /// std::thread::sleep(Duration::from_millis(1));
    /// queue.close();
    /// assert_eq!(task.join().unwrap(), Err(EnqueueError::Closed(3)));
    /// ```
    pub fn enqueue<T>(&self, value: T) -> Result<(), EnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        self.enqueue_sync(value, None)
    }

    /// Tries enqueuing the given value inside the queue with a timeout.
    ///
    /// This method extends [`try_enqueue`](Queue::try_enqueue) by waiting synchronously (with a
    /// timeout) [`SynchronizedNotifier::notify_enqueue`] call, i.e. when a buffer is dequeued, in case of
    /// insufficient capacity.
    ///
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use swap_buffer_queue::SynchronizedQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{EnqueueError, TryEnqueueError};
    /// let queue: Arc<SynchronizedQueue<VecBuffer<usize>>> =
    ///     Arc::new(SynchronizedQueue::with_capacity(1));
    /// queue.try_enqueue(0).unwrap();
    /// assert_eq!(
    ///     queue.enqueue_timeout(1, Duration::from_millis(1)),
    ///     Err(TryEnqueueError::InsufficientCapacity(1))
    /// );
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || {
    ///     std::thread::sleep(Duration::from_millis(1));
    ///     queue_clone.try_dequeue().unwrap();
    /// });
    /// queue.enqueue_timeout(1, Duration::from_secs(1)).unwrap();
    /// ```
    pub fn enqueue_timeout<T>(&self, value: T, timeout: Duration) -> Result<(), TryEnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        self.enqueue_sync(value, Some(Instant::now() + timeout))
    }

    /// Enqueues the given value inside the queue.
    ///
    /// This method extends [`try_enqueue`](Queue::try_enqueue) by waiting asynchronously
    /// [`SynchronizedNotifier::notify_enqueue`] call, i.e. when a buffer is dequeued, in case of
    /// insufficient capacity.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::SynchronizedQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{EnqueueError, TryEnqueueError};
    /// # tokio_test::block_on(async {
    /// let queue: Arc<SynchronizedQueue<VecBuffer<usize>>> =
    ///     Arc::new(SynchronizedQueue::with_capacity(1));
    /// queue.try_enqueue(0).unwrap();
    /// assert_eq!(
    ///     queue.try_enqueue(0),
    ///     Err(TryEnqueueError::InsufficientCapacity(0))
    /// );
    /// // queue is full, let's spawn an enqueuing task and dequeue
    /// let queue_clone = queue.clone();
    /// let task = tokio::spawn(async move { queue_clone.enqueue_async(1).await });
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[0]);
    /// // enqueuing task has succeeded
    /// task.await.unwrap().unwrap();
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[1]);
    /// // let's close the queue
    /// queue.try_enqueue(2).unwrap();
    /// let queue_clone = queue.clone();
    /// let task = tokio::spawn(async move { queue_clone.enqueue_async(3).await });
    /// queue.close();
    /// assert_eq!(task.await.unwrap(), Err(EnqueueError::Closed(3)));
    /// # })
    /// ```
    pub async fn enqueue_async<T>(&self, value: T) -> Result<(), EnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        let mut value = Some(value);
        poll_fn(|cx| {
            let v = value.take().unwrap();
            match try_enqueue(self, v, Some(cx)) {
                Ok(res) => return Poll::Ready(res),
                Err(v) => value.replace(v),
            };
            Poll::Pending
        })
        .await
    }

    fn dequeue_sync(
        &self,
        deadline: Option<Instant>,
    ) -> Result<BufferSlice<B, SynchronizedNotifier>, TryDequeueError> {
        loop {
            if let Some(res) = try_dequeue(self, None) {
                return res;
            }
            if !self.notify().enqueuers.wait_until(deadline) {
                return self.try_dequeue();
            }
        }
    }

    /// Dequeues a buffer with all enqueued values from the queue.
    ///
    /// This method extends [`try_dequeue`](Queue::try_dequeue) by waiting synchronously
    /// [`SynchronizedNotifier::notify_dequeue`] call, i.e. when a value is enqueued, in case of
    /// empty queue.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::SynchronizedQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{DequeueError, TryDequeueError};
    /// let queue: Arc<SynchronizedQueue<VecBuffer<usize>>> =
    ///     Arc::new(SynchronizedQueue::with_capacity(1));
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Empty);
    /// // queue is empty, let's spawn a dequeuing task and enqueue
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || {
    ///     Ok::<_, DequeueError>(queue_clone.dequeue()?.into_iter().collect::<Vec<_>>())
    /// });
    /// queue.try_enqueue(0).unwrap();
    /// // dequeuing task has succeeded
    /// assert_eq!(task.join().unwrap().unwrap().deref(), &[0]);
    /// // let's close the queue
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || {
    ///     Ok::<_, DequeueError>(queue_clone.dequeue()?.into_iter().collect::<Vec<_>>())
    /// });
    /// queue.close();
    /// assert_eq!(task.join().unwrap().unwrap_err(), DequeueError::Closed);
    /// ```
    pub fn dequeue(&self) -> Result<BufferSlice<B, SynchronizedNotifier>, DequeueError> {
        self.dequeue_sync(None).map_err(dequeue_err)
    }

    /// Tries dequeuing a buffer with all enqueued values from the queue with a timeout.
    ///
    /// This method extends [`try_dequeue`](Queue::try_dequeue) by waiting synchronously, with a
    /// timeout, [`SynchronizedNotifier::notify_dequeue`] call, i.e. when a value is enqueued, in case of
    /// empty queue.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use swap_buffer_queue::SynchronizedQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{DequeueError, TryDequeueError};
    /// let queue: Arc<SynchronizedQueue<VecBuffer<usize>>> =
    ///     Arc::new(SynchronizedQueue::with_capacity(1));
    /// assert_eq!(
    ///     queue.dequeue_timeout(Duration::from_millis(1)).unwrap_err(),
    ///     TryDequeueError::Empty
    /// );
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || {
    ///     std::thread::sleep(Duration::from_millis(1));
    ///     queue_clone.try_enqueue(0).unwrap();
    /// });
    /// assert_eq!(
    ///     queue
    ///         .dequeue_timeout(Duration::from_secs(1))
    ///         .unwrap()
    ///         .deref(),
    ///     &[0]
    /// );
    /// ```
    pub fn dequeue_timeout(
        &self,
        timeout: Duration,
    ) -> Result<BufferSlice<B, SynchronizedNotifier>, TryDequeueError> {
        self.dequeue_sync(Some(Instant::now() + timeout))
    }

    /// Dequeues a buffer with all enqueued values from the queue.
    ///
    /// This method extends [`try_dequeue`](Queue::try_dequeue) by waiting asynchronously
    /// [`SynchronizedNotifier::notify_dequeue`] call, i.e. when a value is enqueued, in case of
    /// empty queue.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::SynchronizedQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{DequeueError, TryDequeueError};
    /// # tokio_test::block_on(async {
    /// let queue: Arc<SynchronizedQueue<VecBuffer<usize>>> =
    ///     Arc::new(SynchronizedQueue::with_capacity(1));
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Empty);
    /// // queue is empty, let's spawn a dequeuing task and enqueue
    /// let queue_clone = queue.clone();
    /// let task = tokio::spawn(async move {
    ///     Ok::<_, DequeueError>(
    ///         queue_clone
    ///             .dequeue_async()
    ///             .await?
    ///             .into_iter()
    ///             .collect::<Vec<_>>(),
    ///     )
    /// });
    /// queue.try_enqueue(0).unwrap();
    /// // dequeuing task has succeeded
    /// assert_eq!(task.await.unwrap().unwrap().deref(), &[0]);
    /// // let's close the queue
    /// let queue_clone = queue.clone();
    /// let task = tokio::spawn(async move {
    ///     Ok::<_, DequeueError>(
    ///         queue_clone
    ///             .dequeue_async()
    ///             .await?
    ///             .into_iter()
    ///             .collect::<Vec<_>>(),
    ///     )
    /// });
    /// queue.close();
    /// assert_eq!(task.await.unwrap().unwrap_err(), DequeueError::Closed);
    /// # })
    /// ```
    pub async fn dequeue_async(
        &self,
    ) -> Result<BufferSlice<B, SynchronizedNotifier>, DequeueError> {
        poll_fn(|cx| {
            if let Some(res) = try_dequeue(self, Some(cx)) {
                return Poll::Ready(res.map_err(dequeue_err));
            }
            Poll::Pending
        })
        .await
    }
}

impl<B> Queue<B, SynchronizedNotifier>
where
    B: Buffer + Drain,
{
    /// Returns an iterator over the element of the queue (see [`BufferIter`](crate::buffer::BufferIter)).
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SynchronizedQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SynchronizedQueue<VecBuffer<usize>> = SynchronizedQueue::with_capacity(42);
    /// queue.try_enqueue(0).unwrap();
    /// queue.try_enqueue(1).unwrap();
    ///
    /// let mut iter = queue.iter();
    /// assert_eq!(iter.next(), Some(0));
    /// drop(iter);
    /// let mut iter = queue.iter();
    /// assert_eq!(iter.next(), Some(1));
    /// queue.close(); // close in order to stop the iterator
    /// assert_eq!(iter.next(), None);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = B::Value> + '_ {
        iter::repeat_with(|| self.dequeue())
            .map_while(|res| res.ok())
            .flatten()
    }

    #[cfg(feature = "futures")]
    /// Returns an stream over the element of the queue (see [`BufferIter`](crate::buffer::BufferIter)).
    ///
    /// # Examples
    /// ```
    /// # use futures::StreamExt;
    /// # use swap_buffer_queue::SynchronizedQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # tokio_test::block_on(async {
    /// let queue: SynchronizedQueue<VecBuffer<usize>> = SynchronizedQueue::with_capacity(42);
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
    pub fn stream(&self) -> impl futures::Stream<Item = B::Value> + '_ {
        use futures::{stream, StreamExt};
        stream::repeat_with(|| stream::once(self.dequeue_async()))
            .flatten()
            .take_while(|res| {
                let is_ok = res.is_ok();
                async move { is_ok }
            })
            .flat_map(|res| stream::iter(res.unwrap()))
    }
}

fn try_enqueue<B, T>(
    queue: &Queue<B, SynchronizedNotifier>,
    mut value: T,
    cx: Option<&Context>,
) -> Result<Result<(), TryEnqueueError<T>>, T>
where
    B: Buffer,
    T: BufferValue<B>,
{
    for _ in 0..SPIN_LIMIT {
        match queue.try_enqueue(value) {
            Err(TryEnqueueError::InsufficientCapacity(v)) if v.size().get() <= queue.capacity() => {
                value = v;
            }
            res => return Ok(res),
        };
        std::hint::spin_loop();
    }
    queue.notify().enqueuers.register(cx);
    match queue.try_enqueue(value) {
        Err(TryEnqueueError::InsufficientCapacity(v)) if v.size().get() <= queue.capacity() => {
            Err(v)
        }
        res => Ok(res),
    }
}

fn try_dequeue<'a, B>(
    queue: &'a Queue<B, SynchronizedNotifier>,
    cx: Option<&Context>,
) -> Option<Result<BufferSlice<'a, B, SynchronizedNotifier>, TryDequeueError>>
where
    B: Buffer,
{
    for _ in 0..SPIN_LIMIT {
        match queue.try_dequeue() {
            Err(TryDequeueError::Empty | TryDequeueError::Pending) => {}
            res => return Some(res),
        }
        std::hint::spin_loop();
    }
    queue.notify().dequeuers.register(cx);
    match queue.try_dequeue() {
        Err(TryDequeueError::Empty | TryDequeueError::Pending) => None,
        res => Some(res),
    }
}

fn dequeue_err(error: TryDequeueError) -> DequeueError {
    match error {
        TryDequeueError::Closed => DequeueError::Closed,
        TryDequeueError::Conflict => DequeueError::Conflict,
        _ => unreachable!(),
    }
}
