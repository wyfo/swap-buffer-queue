//! Synchronous implementation of [`SBQueue`].
use std::{
    iter,
    time::{Duration, Instant},
};

use crossbeam_utils::CachePadded;

use crate::{
    buffer::{Buffer, BufferSlice, BufferValue, Drain},
    error::{DequeueError, EnqueueError, TryDequeueError, TryEnqueueError},
    loom::{AtomicBool, Condvar, Mutex, Ordering},
    notify::Notify,
    queue::SBQueue,
};

/// A synchronous notifier.
#[derive(Debug, Default)]
pub struct SyncNotifier<const EAGER: bool = true> {
    cond_var: Condvar,
    lock: Mutex<()>,
    dequeue_waiting: CachePadded<AtomicBool>,
    enqueue_waiting: CachePadded<AtomicBool>,
}

impl<const EAGER: bool> SyncNotifier<EAGER> {
    fn wait_until(&self, waiting: &AtomicBool, deadline: Option<Instant>) -> bool {
        let mut lock = self.lock.lock().unwrap();
        while waiting.load(Ordering::SeqCst) {
            lock = match deadline.map(|d| d.checked_duration_since(Instant::now())) {
                Some(Some(timeout)) => self.cond_var.wait_timeout(lock, timeout).unwrap().0,
                Some(None) => return false,
                None => self.cond_var.wait(lock).unwrap(),
            };
        }
        true
    }

    fn notify(&self, waiting: &AtomicBool) {
        if waiting.load(Ordering::Relaxed)
            && waiting
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
        {
            let _lock = self.lock.lock().unwrap();
            self.cond_var.notify_all();
        }
    }
}

impl<const EAGER: bool> Notify for SyncNotifier<EAGER> {
    fn notify_dequeue(&self, may_be_ready: bool) {
        if EAGER || may_be_ready {
            self.notify(&self.dequeue_waiting);
        }
    }

    fn notify_enqueue(&self) {
        self.notify(&self.enqueue_waiting);
    }
}

impl<B, const EAGER: bool> SBQueue<B, SyncNotifier<EAGER>>
where
    B: Buffer,
{
    fn enqueue_internal<T>(
        &self,
        mut value: T,
        deadline: Option<Instant>,
    ) -> Result<(), TryEnqueueError<T>>
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
            if !self
                .notify()
                .wait_until(&self.notify().enqueue_waiting, deadline)
            {
                return self.try_enqueue(value);
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
        self.enqueue_internal(value, Some(Instant::now() + timeout))
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
    /// # use std::time::Duration;
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
        self.enqueue_internal(value, None)
    }

    fn dequeue_internal(
        &self,
        deadline: Option<Instant>,
    ) -> Result<BufferSlice<B, SyncNotifier<EAGER>>, TryDequeueError> {
        loop {
            match self.try_dequeue() {
                Err(TryDequeueError::Empty | TryDequeueError::Pending) => {}
                res => return res,
            }
            self.notify().dequeue_waiting.store(true, Ordering::SeqCst);
            match self.try_dequeue() {
                Err(TryDequeueError::Empty | TryDequeueError::Pending) => {}
                res => return res,
            }
            if !self
                .notify()
                .wait_until(&self.notify().dequeue_waiting, deadline)
            {
                return self.try_dequeue();
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
    ) -> Result<BufferSlice<B, SyncNotifier<EAGER>>, TryDequeueError> {
        self.dequeue_internal(Some(Instant::now() + timeout))
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
    pub fn dequeue(&self) -> Result<BufferSlice<B, SyncNotifier<EAGER>>, DequeueError> {
        match self.dequeue_internal(None) {
            Ok(buf) => Ok(buf),
            Err(TryDequeueError::Closed) => Err(DequeueError::Closed),
            Err(TryDequeueError::Conflict) => Err(DequeueError::Conflict),
            Err(TryDequeueError::Empty | TryDequeueError::Pending) => unreachable!(),
        }
    }
}

impl<B, const EAGER: bool> SBQueue<B, SyncNotifier<EAGER>>
where
    B: Buffer + Drain,
{
    /// Returns an iterator over the element of the queue (see [`BufferIter`]).
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SyncSBQueue<VecBuffer<usize>> = SyncSBQueue::with_capacity(42);
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
}

#[cfg(test)]
mod test {
    use std::{ops::Deref, sync::Arc, time::Duration};

    use crate::{
        buffer::VecBuffer,
        error::{EnqueueError, TryEnqueueError},
        SyncSBQueue,
    };

    #[test]
    fn plop() {
        let queue: Arc<SyncSBQueue<VecBuffer<usize>>> = Arc::new(SyncSBQueue::with_capacity(1));
        queue.try_enqueue(0).unwrap();
        assert_eq!(
            queue.try_enqueue(1),
            Err(TryEnqueueError::InsufficientCapacity(1))
        );
        // queue is full, let's spawn an enqueuing task and dequeue
        let queue_clone = queue.clone();
        let task = std::thread::spawn(move || queue_clone.enqueue(1));
        let queue_clone = queue.clone();
        let task2 = std::thread::spawn(move || queue_clone.enqueue(1));
        std::thread::sleep(Duration::from_millis(1));
        println!("1");
        assert_eq!(queue.dequeue().unwrap().deref(), &[0]);
        // enqueuing task has succeeded
        // task.join().unwrap().unwrap();
        println!("2");
        assert_eq!(queue.dequeue().unwrap().deref(), &[1]);
        task.join().unwrap().unwrap();
        task2.join().unwrap().unwrap();
        println!("3");
        assert_eq!(queue.dequeue().unwrap().deref(), &[1]);
        // let's close the queue
        queue.try_enqueue(2).unwrap();
        let queue_clone = queue.clone();
        let task = std::thread::spawn(move || queue_clone.enqueue(3));
        std::thread::sleep(Duration::from_millis(1));
        queue.close();
        assert_eq!(task.join().unwrap(), Err(EnqueueError::Closed(3)));
    }
}
