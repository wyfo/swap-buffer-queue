use std::{fmt, hint, ops::Range};

use crossbeam_utils::CachePadded;

use crate::{
    buffer::{Buffer, BufferSlice, BufferValue, BufferWithLen, Drain, Resize},
    error::{TryDequeueError, TryEnqueueError},
    loom::{AtomicUsize, Ordering},
    notify::Notify,
};

const CLOSED_FLAG: usize = (usize::MAX >> 1) + 1;
#[cfg(not(loom))]
const SPIN_LIMIT: i32 = 100;
#[cfg(loom)]
const SPIN_LIMIT: i32 = 1;

/// A buffered MPSC "swap-buffer" queue.
pub struct SBQueue<B, N = ()>
where
    B: Buffer,
{
    buffer_remain: CachePadded<AtomicUsize>,
    pending_dequeue: CachePadded<AtomicUsize>,
    buffers: [BufferWithLen<B>; 2],
    notify: N,
}

impl<B, N> SBQueue<B, N>
where
    B: Buffer,
    N: Default,
{
    /// Create a new queue using buffer default.
    ///
    /// Buffer default may have a non-zero capacity, e.g. array buffer.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::new();
    /// ```
    pub fn new() -> Self {
        let buffers: [BufferWithLen<B>; 2] = Default::default();
        let capacity = buffers[0].capacity();
        Self {
            buffer_remain: AtomicUsize::new(capacity << 1).into(),
            pending_dequeue: AtomicUsize::new(0).into(),
            buffers,
            notify: Default::default(),
        }
    }
}

impl<B, N> SBQueue<B, N>
where
    B: Buffer + Resize,
    N: Default,
{
    /// Creates a new queue with the given capacity.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer_remain: AtomicUsize::new(capacity << 1).into(),
            pending_dequeue: AtomicUsize::new(0).into(),
            buffers: [
                BufferWithLen::with_capacity(capacity),
                BufferWithLen::with_capacity(capacity),
            ],
            notify: Default::default(),
        }
    }
}

impl<B, N> SBQueue<B, N>
where
    B: Buffer,
{
    /// Returns queue's [`Notify`] implementor.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::AsyncSBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// use swap_buffer_queue::notify::Notify;
    ///
    /// let queue: AsyncSBQueue<VecBuffer<usize>> = AsyncSBQueue::with_capacity(42);
    /// queue.notify().notify_dequeue(true);
    /// ```
    pub fn notify(&self) -> &N {
        &self.notify
    }

    fn current_buffer(&self) -> &BufferWithLen<B> {
        &self.buffers[self.buffer_remain.load(Ordering::Acquire) & 1]
    }

    /// Returns the current buffer capacity.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// assert_eq!(queue.capacity(), 42);
    /// ```
    pub fn capacity(&self) -> usize {
        self.current_buffer().capacity()
    }

    /// Returns the current buffer length.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// assert_eq!(queue.len(), 0);
    /// queue.try_enqueue(0).unwrap();
    /// assert_eq!(queue.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.current_buffer().len()
    }

    /// Returns `true` if the current buffer is empty.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// assert!(queue.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the queue is closed.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// assert!(!queue.is_closed());
    /// queue.close();
    /// assert!(queue.is_closed());
    /// ```
    pub fn is_closed(&self) -> bool {
        self.buffer_remain.load(Ordering::Acquire) & CLOSED_FLAG != 0
    }

    /// Reopen a closed queue.
    ///
    /// Calling this method when the queue is not closed has no effect.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// queue.close();
    /// assert!(queue.is_closed());
    /// queue.reopen();
    /// assert!(!queue.is_closed());
    /// ```
    pub fn reopen(&self) {
        self.buffer_remain.fetch_and(!CLOSED_FLAG, Ordering::AcqRel);
    }

    fn try_dequeue_spin(&self, buffer_index: usize, len: usize) -> Option<BufferSlice<B, N>> {
        assert_ne!(len, 0);
        let buffer = &self.buffers[buffer_index];
        for _ in 0..SPIN_LIMIT {
            let buffer_len = buffer.len();
            if buffer_len >= len {
                let range = buffer_len - len..buffer_len;
                let slice = unsafe { buffer.slice(range.clone()) };
                return Some(BufferSlice::new(self, buffer_index, range, slice));
            }
            hint::spin_loop();
        }
        self.pending_dequeue
            .store(buffer_index | (len << 1), Ordering::Release);
        None
    }

    pub(crate) fn release(&self, buffer_index: usize, range: Range<usize>) {
        unsafe { self.buffers[buffer_index].clear(range) };
        self.pending_dequeue
            .store(!buffer_index & 1, Ordering::Release);
    }

    pub(crate) fn requeue(&self, buffer_index: usize, range: Range<usize>) {
        if range.is_empty() {
            self.release(buffer_index, range);
        } else {
            self.pending_dequeue.store(
                buffer_index | ((range.end - range.start) << 1),
                Ordering::Release,
            );
        }
    }
}

impl<B, N> SBQueue<B, N>
where
    B: Buffer,
    N: Notify,
{
    /// Tries enqueuing the given value into the queue.
    ///
    /// Enqueuing will fail if the queue has insufficient capacity, or if it is closed. In case of
    /// success, it will notify waiting dequeuing operations using [`Notify::notify_dequeue`].
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::TryEnqueueError;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(1);
    /// queue.try_enqueue(0).unwrap();
    /// // queue is full
    /// assert_eq!(
    ///     queue.try_enqueue(0),
    ///     Err(TryEnqueueError::InsufficientCapacity(0))
    /// );
    /// // let's close the queue
    /// queue.close();
    /// assert_eq!(queue.try_enqueue(0), Err(TryEnqueueError::Closed(0)));
    /// ```
    pub fn try_enqueue<T>(&self, value: T) -> Result<(), TryEnqueueError<T>>
    where
        T: BufferValue<B>,
    {
        let shifted_size = value.size() << 1;
        let mut buffer_remain = self.buffer_remain.load(Ordering::Acquire);
        loop {
            if buffer_remain & CLOSED_FLAG != 0 {
                return Err(TryEnqueueError::Closed(value));
            }
            if buffer_remain < shifted_size {
                return Err(TryEnqueueError::InsufficientCapacity(value));
            }
            match self.buffer_remain.compare_exchange_weak(
                buffer_remain,
                buffer_remain - shifted_size,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(s) => buffer_remain = s,
            }
        }
        let buffer = &self.buffers[buffer_remain & 1];
        let index = buffer.capacity() - (buffer_remain >> 1);
        let may_be_ready = unsafe { buffer.insert(index, value) };
        self.notify.notify_dequeue(may_be_ready);
        Ok(())
    }

    fn try_dequeue_internal(
        &self,
        resize: Option<impl FnOnce(&BufferWithLen<B>) -> usize>,
        insert: Option<impl FnOnce(&BufferWithLen<B>) -> usize>,
    ) -> Result<BufferSlice<B, N>, TryDequeueError> {
        let pending_dequeue = self.pending_dequeue.swap(usize::MAX, Ordering::AcqRel);
        if pending_dequeue == usize::MAX {
            return Err(TryDequeueError::Conflict);
        }
        let buffer_index = pending_dequeue & 1;
        if pending_dequeue >> 1 != 0 {
            return self
                .try_dequeue_spin(buffer_index, pending_dequeue >> 1)
                .ok_or(TryDequeueError::Pending);
        }
        let mut buffer_remain = self.buffer_remain.load(Ordering::Acquire);
        assert_eq!(buffer_index, buffer_remain & 1);
        let capacity = self.buffers[buffer_index].capacity();
        let swap_if_empty = resize.is_some() || insert.is_some();
        if ((buffer_remain & !CLOSED_FLAG) >> 1) == capacity {
            if buffer_remain & CLOSED_FLAG != 0 {
                self.pending_dequeue
                    .store(pending_dequeue, Ordering::Release);
                return Err(TryDequeueError::Closed);
            } else if !swap_if_empty {
                self.pending_dequeue
                    .store(pending_dequeue, Ordering::Release);
                return Err(TryDequeueError::Empty);
            }
        }
        let next_buffer_index = !buffer_remain & 1;
        let next_buffer = &self.buffers[next_buffer_index];
        let mut next_capa = resize.map_or_else(|| next_buffer.capacity(), |r| r(next_buffer));
        if let Some(insert) = insert {
            next_capa = insert(next_buffer);
        }
        let next_buffer_remain = next_buffer_index | (next_capa << 1);
        while let Err(s) = self.buffer_remain.compare_exchange_weak(
            buffer_remain,
            next_buffer_remain | (buffer_remain & CLOSED_FLAG),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            buffer_remain = s;
        }
        self.notify.notify_enqueue();
        let len = capacity - ((buffer_remain & !CLOSED_FLAG) >> 1);
        if swap_if_empty && len == 0 {
            self.pending_dequeue
                .store(!buffer_index & 1, Ordering::Release);
            return Err(TryDequeueError::Empty);
        }
        self.try_dequeue_spin(buffer_index, len)
            .ok_or(TryDequeueError::Pending)
    }

    /// Tries dequeuing a buffer with all enqueued values from the queue.
    ///
    /// This method swaps the current buffer with the other one, which is empty. All concurrent
    /// enqueuing must end before the the current buffer is really dequeuable, so the queue may
    /// be in a transitory state where `try_dequeue` must be retried. In this state, after a spin
    /// loop, this method will return a [`TryDequeueError::Pending`] error.
    ///
    /// Dequeuing also fails if the queue is empty, or if it is closed. Moreover, as the algorithm
    /// is MPSC, dequeuing is protected against concurrent calls, failing with
    /// [`TryDequeueError::Conflict`] error.
    ///
    /// It returns a [`BufferSlice`], which holds, as its name may indicate, a reference to the
    /// dequeued buffer. That's why, the concurrent dequeuing protection is maintained for the
    /// lifetime of the buffer slice.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::TryDequeueError;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// queue.try_enqueue(0).unwrap();
    /// queue.try_enqueue(1).unwrap();
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[0, 1]);
    /// // dequeuing cannot be done concurrently (`slice` is still in scope)
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Conflict);
    /// drop(slice);
    /// // let's close the queue
    /// queue.try_enqueue(2).unwrap();
    /// queue.close();
    /// // queue can be dequeued while closed when not empty
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[2]);
    /// drop(slice);
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Closed)
    /// ```
    pub fn try_dequeue(&self) -> Result<BufferSlice<B, N>, TryDequeueError> {
        self.try_dequeue_internal(
            None::<&dyn Fn(&BufferWithLen<B>) -> usize>,
            None::<&dyn Fn(&BufferWithLen<B>) -> usize>,
        )
    }

    /// Closes the queue.
    ///
    /// Closed queue can no more accept enqueuing, but it can be dequeued while not empty.
    /// Calling this method on a closed queue has no effect.
    /// See [`reopen`](SBQueue::reopen) to reopen a closed queue.
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{TryDequeueError, TryEnqueueError};
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// queue.try_enqueue(0).unwrap();
    /// queue.close();
    /// assert!(queue.is_closed());
    /// assert_eq!(queue.try_enqueue(1), Err(TryEnqueueError::Closed(1)));
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[0]);
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Closed);
    /// ```
    pub fn close(&self) {
        self.buffer_remain.fetch_or(CLOSED_FLAG, Ordering::AcqRel);
        self.notify.notify_dequeue(true);
        self.notify.notify_enqueue();
    }
}

impl<B, N> SBQueue<B, N>
where
    B: Buffer + Resize,
    N: Notify,
{
    /// Tries dequeuing a buffer with all enqueued values from the queue, and resizes the next
    /// buffer to be used for enqueuing.
    ///
    /// This method is an extension of [`try_dequeue`](SBQueue::try_dequeue) method. In fact,
    /// before swapping the buffers, next one is empty and protected, so it can be resized, and
    /// it is also possible to add values in it before making it available for enqueuing.
    /// This can be used to make the queue [unbounded](SBQueue#an-amortized-unbounded-recipe).
    ///
    /// It is worth to be noted that only one buffer is resized, so it can lead to asymmetric buffers.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::TryEnqueueError;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(1);
    /// queue.try_enqueue(0).unwrap();
    /// // queue is full
    /// assert_eq!(
    ///     queue.try_enqueue(1),
    ///     Err(TryEnqueueError::InsufficientCapacity(1))
    /// );
    /// // dequeue and resize, inserting elements before the buffer is available
    /// let slice = queue
    ///     .try_dequeue_and_resize(3, Some(std::iter::once(42)))
    ///     .unwrap();
    /// assert_eq!(slice.deref(), &[0]);
    /// drop(slice);
    /// // capacity has been increased
    /// queue.try_enqueue(1).unwrap();
    /// queue.try_enqueue(2).unwrap();
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[42, 1, 2]);
    /// ```
    ///
    /// ## An amortized unbounded recipe
    ///
    /// ```rust
    /// # use std::ops::Deref;
    /// # use std::sync::Mutex;
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::{BufferSlice, BufferValue, VecBuffer};
    /// # use swap_buffer_queue::error::{EnqueueError, TryDequeueError, TryEnqueueError};
    /// # use swap_buffer_queue::notify::Notify;
    /// fn enqueue_unbounded<T: BufferValue<VecBuffer<T>>>(
    ///     queue: &SBQueue<VecBuffer<T>>,
    ///     overflow: &Mutex<Vec<T>>,
    ///     mut value: T,
    /// ) -> Result<(), EnqueueError<T>> {
    ///     // first, try to enqueue normally
    ///     match queue.try_enqueue(value) {
    ///         Err(TryEnqueueError::InsufficientCapacity(v)) => value = v,
    ///         res => return res,
    ///     };
    ///     // if the enqueuing fails, lock the overflow
    ///     let mut guard = overflow.lock().unwrap();
    ///     // retry to enqueue (we never know what happened during lock acquisition)
    ///     match queue.try_enqueue(value) {
    ///         Err(TryEnqueueError::InsufficientCapacity(v)) => value = v,
    ///         res => return res,
    ///     };
    ///     // then push the values to the overflow vector
    ///     guard.push(value);
    ///     // notify possible waiting dequeue
    ///     queue.notify().notify_dequeue(true);
    ///     Ok(())
    /// }
    ///
    /// fn try_dequeue_unbounded<'a, T>(
    ///     queue: &'a SBQueue<VecBuffer<T>>,
    ///     overflow: &Mutex<Vec<T>>,
    /// ) -> Result<BufferSlice<'a, VecBuffer<T>, ()>, TryDequeueError> {
    ///     // lock the overflow and use `try_dequeue_and_resize` to drain the overflow into the
    ///     // queue
    ///     let mut guard = overflow.lock().unwrap();
    ///     queue.try_dequeue_and_resize(queue.capacity() + guard.len(), Some(guard.drain(..)))
    /// }
    ///
    /// // queue is initialized with zero capacity
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::new();
    /// let overflow = Mutex::new(Vec::new());
    /// assert_eq!(queue.capacity(), 0);
    /// enqueue_unbounded(&queue, &overflow, 0).unwrap();
    /// assert_eq!(queue.capacity(), 0);
    /// assert_eq!(
    ///     try_dequeue_unbounded(&queue, &overflow).unwrap_err(),
    ///     TryDequeueError::Empty
    /// );
    /// assert_eq!(queue.capacity(), 1);
    /// assert_eq!(queue.len(), 1);
    /// enqueue_unbounded(&queue, &overflow, 1).unwrap();
    /// assert_eq!(queue.capacity(), 1);
    /// assert_eq!(queue.len(), 1);
    /// assert_eq!(overflow.lock().unwrap().len(), 1);
    /// assert_eq!(
    ///     try_dequeue_unbounded(&queue, &overflow).unwrap().deref(),
    ///     &[0]
    /// );
    /// assert_eq!(queue.capacity(), 2);
    /// assert_eq!(queue.len(), 1);
    /// enqueue_unbounded(&queue, &overflow, 2).unwrap();
    /// assert_eq!(
    ///     try_dequeue_unbounded(&queue, &overflow).unwrap().deref(),
    ///     &[1, 2]
    /// );
    /// ```
    pub fn try_dequeue_and_resize<T>(
        &self,
        capacity: impl Into<Option<usize>>,
        insert: Option<impl Iterator<Item = T>>,
    ) -> Result<BufferSlice<B, N>, TryDequeueError>
    where
        T: BufferValue<B>,
    {
        let capacity = capacity.into();
        self.try_dequeue_internal(
            capacity.map(|capa| {
                move |buf: &BufferWithLen<B>| {
                    unsafe { buf.resize(capa) };
                    capa
                }
            }),
            insert.map(|insert| {
                |buf: &BufferWithLen<B>| {
                    let mut next_capa = buf.capacity();
                    for (i, value) in insert.enumerate() {
                        let value_size = value.size();
                        if value_size > buf.capacity() {
                            break;
                        }
                        unsafe { buf.insert(i, value) };
                        next_capa -= value_size;
                    }
                    next_capa
                }
            }),
        )
    }
}

impl<B, N> SBQueue<B, N>
where
    B: Buffer + Drain,
{
    pub(crate) fn remove(&self, buffer_index: usize, index: usize) -> (B::Value, usize) {
        unsafe { self.buffers[buffer_index].remove(index) }
    }
}

impl<B, N> Default for SBQueue<B, N>
where
    B: Buffer,
    N: Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<B, N> fmt::Debug for SBQueue<B, N>
where
    B: Buffer,
    N: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug_struct = f.debug_struct("SBQueue");
        self.buffers[self.buffer_remain.load(Ordering::Acquire) & 1].debug(&mut debug_struct);
        debug_struct.field("notify", &self.notify);
        debug_struct.finish()
    }
}

impl<B, N> Drop for SBQueue<B, N>
where
    B: Buffer,
{
    fn drop(&mut self) {
        let pending_dequeue = self.pending_dequeue.swap(usize::MAX, Ordering::Relaxed);
        let buffer_index = pending_dequeue & 1;
        if pending_dequeue != usize::MAX && pending_dequeue >> 1 != 0 {
            self.try_dequeue_spin(buffer_index, pending_dequeue >> 1);
        }
    }
}
