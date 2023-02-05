//! Synchronous implementation of [`SBQueue`].

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};

use crate::{
    buffer::{Buffer, BufferSlice, BufferValue},
    error::{DequeueError, EnqueueError, TryDequeueError, TryEnqueueError},
    notify::Notify,
    queue::SBQueue,
};

/// A synchronous notifier.
#[derive(Debug, Default)]
pub struct SyncNotifier<const EAGER: bool = true> {
    cond_var: Condvar,
    lock: Mutex<()>,
    enqueue_waiting: AtomicBool,
    dequeue_waiting: AtomicBool,
}

impl<const EAGER: bool> Notify for SyncNotifier<EAGER> {
    fn notify_dequeue(&self, may_be_ready: bool) {
        if (EAGER || may_be_ready) && self.dequeue_waiting.swap(false, Ordering::Relaxed) {
            self.cond_var.notify_all();
        }
    }

    fn notify_enqueue(&self) {
        if self.enqueue_waiting.swap(false, Ordering::Relaxed) {
            self.cond_var.notify_all();
        }
    }
}

impl<B> SBQueue<B, SyncNotifier>
where
    B: Buffer,
{
    fn wait_until<'a>(
        &self,
        lock: MutexGuard<'a, ()>,
        start: Instant,
        timeout: Option<&mut Duration>,
    ) -> Option<MutexGuard<'a, ()>> {
        Some(if let Some(timeout) = timeout {
            match timeout.checked_sub(start.elapsed()) {
                Some(t) => *timeout = t,
                None => return None,
            }
            self.notify()
                .cond_var
                .wait_timeout(lock, *timeout)
                .unwrap()
                .0
        } else {
            self.notify().cond_var.wait(lock).unwrap()
        })
    }

    fn enqueue_internal<T>(
        &self,
        mut value: T,
        mut timeout: Option<Duration>,
    ) -> Result<(), TryEnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        match self.try_enqueue(value) {
            Err(TryEnqueueError::InsufficientCapacity(v)) if v.size() <= self.capacity() => {
                value = v
            }
            res => return res,
        };
        let mut lock = self.notify().lock.lock().unwrap();
        let start = Instant::now();
        loop {
            match self.try_enqueue(value) {
                Err(TryEnqueueError::InsufficientCapacity(v)) => value = v,
                res => return res,
            };
            self.notify().enqueue_waiting.store(true, Ordering::Relaxed);
            match self.wait_until(lock, start, timeout.as_mut()) {
                Some(l) => lock = l,
                None => return Err(TryEnqueueError::InsufficientCapacity(value)),
            }
        }
    }

    /// Tries enqueuing the given value inside the queue with a timeout.
    ///
    /// This method extends [`try_enqueue`](SBQueue::try_enqueue) by waiting synchronously, with a
    /// timeout, [`SyncNotifier::notify_enqueue`] call, i.e. when a buffer is dequeued, in case of
    /// insufficient capacity.
    ///
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use swap_buffer_queue::SyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{EnqueueError, TryEnqueueError};
    /// let queue: Arc<SyncSBQueue<VecBuffer<usize>>> = Arc::new(SyncSBQueue::with_capacity(1));
    /// queue.try_enqueue(0).unwrap();
    /// assert_eq!(
    ///     queue.try_enqueue_timeout(1, Duration::from_millis(1)),
    ///     Err(TryEnqueueError::InsufficientCapacity(1))
    /// );
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || {
    ///     std::thread::sleep(Duration::from_millis(1));
    ///     queue_clone.try_dequeue().unwrap();
    /// });
    /// queue
    ///     .try_enqueue_timeout(1, Duration::from_secs(1))
    ///     .unwrap();
    /// ```
    pub fn try_enqueue_timeout<T>(
        &self,
        value: T,
        timeout: Duration,
    ) -> Result<(), TryEnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        self.enqueue_internal(value, Some(timeout))
    }

    /// Enqueues the given value inside the queue.
    ///
    /// This method extends [`try_enqueue`](SBQueue::try_enqueue) by waiting synchronously
    /// [`SyncNotifier::notify_enqueue`] call, i.e. when a buffer is dequeued, in case of
    /// insufficient capacity.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::SyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{EnqueueError, TryEnqueueError};
    /// let queue: Arc<SyncSBQueue<VecBuffer<usize>>> = Arc::new(SyncSBQueue::with_capacity(1));
    /// queue.try_enqueue(0).unwrap();
    /// assert_eq!(
    ///     queue.try_enqueue(1),
    ///     Err(TryEnqueueError::InsufficientCapacity(1))
    /// );
    /// // queue is full, let's spawn an enqueuing task and dequeue
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || queue_clone.enqueue(1));
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[0]);
    /// // enqueuing task has succeeded
    /// task.join().unwrap().unwrap();
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[1]);
    /// // let's close the queue
    /// queue.try_enqueue(2).unwrap();
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || queue_clone.enqueue(3));
    /// queue.close();
    /// assert_eq!(task.join().unwrap(), Err(EnqueueError::Closed(3)));
    /// ```
    pub fn enqueue<T>(&self, value: T) -> Result<(), EnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        self.enqueue_internal(value, None)
    }

    fn dequeue_internal(
        &self,
        mut timeout: Option<Duration>,
    ) -> Result<BufferSlice<B, SyncNotifier>, TryDequeueError> {
        match self.try_dequeue() {
            Err(TryDequeueError::Empty | TryDequeueError::Pending) => {}
            res => return res,
        }
        let mut lock = self.notify().lock.lock().unwrap();
        let start = Instant::now();
        loop {
            let err = match self.try_dequeue() {
                Err(err @ (TryDequeueError::Empty | TryDequeueError::Pending)) => err,
                res => return res,
            };
            self.notify().dequeue_waiting.store(true, Ordering::Relaxed);
            match self.wait_until(lock, start, timeout.as_mut()) {
                Some(l) => lock = l,
                None => return Err(err),
            }
        }
    }

    /// Tries dequeuing a buffer with all enqueued values from the queue with a timeout.
    ///
    /// This method extends [`try_dequeue`](SBQueue::try_dequeue) by waiting synchronously, with a
    /// timeout, [`SyncNotifier::notify_dequeue`] call, i.e. when a value is enqueued, in case of
    /// empty queue.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use swap_buffer_queue::SyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{DequeueError, TryDequeueError};
    /// let queue: Arc<SyncSBQueue<VecBuffer<usize>>> = Arc::new(SyncSBQueue::with_capacity(1));
    /// assert_eq!(
    ///     queue
    ///         .try_dequeue_timeout(Duration::from_millis(1))
    ///         .unwrap_err(),
    ///     TryDequeueError::Empty
    /// );
    /// let queue_clone = queue.clone();
    /// let task = std::thread::spawn(move || {
    ///     std::thread::sleep(Duration::from_millis(1));
    ///     queue_clone.try_enqueue(0).unwrap();
    /// });
    /// assert_eq!(
    ///     queue
    ///         .try_dequeue_timeout(Duration::from_secs(1))
    ///         .unwrap()
    ///         .deref(),
    ///     &[0]
    /// );
    /// ```
    pub fn try_dequeue_timeout(
        &self,
        timeout: Duration,
    ) -> Result<BufferSlice<B, SyncNotifier>, TryDequeueError> {
        self.dequeue_internal(Some(timeout))
    }

    /// Dequeues a buffer with all enqueued values from the queue.
    ///
    /// This method extends [`try_dequeue`](SBQueue::try_dequeue) by waiting synchronously
    /// [`SyncNotifier::notify_dequeue`] call, i.e. when a value is enqueued, in case of
    /// empty queue.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::SyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{DequeueError, TryDequeueError};
    /// let queue: Arc<SyncSBQueue<VecBuffer<usize>>> = Arc::new(SyncSBQueue::with_capacity(1));
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
    pub fn dequeue(&self) -> Result<BufferSlice<B, SyncNotifier>, DequeueError> {
        match self.dequeue_internal(None) {
            Ok(buf) => Ok(buf),
            Err(TryDequeueError::Closed) => Err(DequeueError::Closed),
            Err(TryDequeueError::Conflict) => Err(DequeueError::Conflict),
            Err(TryDequeueError::Empty | TryDequeueError::Pending) => unreachable!(),
        }
    }
}
