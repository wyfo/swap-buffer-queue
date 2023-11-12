use std::{fmt, num::NonZeroUsize, ops::Range};

use crossbeam_utils::CachePadded;

use crate::{
    buffer::{Buffer, BufferSlice, Drain, InsertIntoBuffer, Resize},
    error::{TryDequeueError, TryEnqueueError},
    loom::{
        sync::atomic::{AtomicUsize, Ordering},
        LoomUnsafeCell, BACKOFF_LIMIT, SPIN_LIMIT,
    },
    notify::Notify,
};

const CLOSED_FLAG: usize = (usize::MAX >> 1) + 1;
const DEQUEUING_LOCKED: usize = usize::MAX;

/// Atomic usize with the following (64bit) representation
/// 64------------63---------------------1--------------0
/// | closed flag |  enqueuing capacity  | buffer index |
/// +-------------+----------------------+--------------+
/// *buffer index* bit is the index (0 or 1) of the enqueuing buffer
/// *enqueuing capacity* is the remaining enqueuing capacity, starting at the capacity of the
/// buffer, and decreasing until zero
/// *closed flag* is a bit flag to mark the queue as closed
#[derive(Copy, Clone)]
#[repr(transparent)]
struct EnqueuingCapacity(usize);

impl EnqueuingCapacity {
    #[inline]
    fn new(buffer_index: usize, capacity: usize) -> Self {
        assert!(capacity << 1 < CLOSED_FLAG);
        Self(buffer_index | (capacity << 1))
    }

    #[inline] // I've found compiler not inlining this function
    fn buffer_index(self) -> usize {
        self.0 & 1
    }

    #[inline]
    fn remaining_capacity(self) -> usize {
        (self.0 & !CLOSED_FLAG) >> 1
    }

    #[inline]
    fn is_closed(self) -> bool {
        self.0 & CLOSED_FLAG != 0
    }

    #[inline]
    fn try_reserve(self, size: NonZeroUsize) -> Option<Self> {
        self.0.checked_sub(size.get() << 1).map(Self)
    }

    #[inline]
    fn with_closed(self, enqueuing: Self) -> Self {
        Self(self.0 | (enqueuing.0 & CLOSED_FLAG))
    }

    #[inline]
    fn from_atomic(atomic: usize) -> Self {
        Self(atomic)
    }

    #[inline]
    fn into_atomic(self) -> usize {
        self.0
    }

    #[inline]
    fn close(atomic: &AtomicUsize, ordering: Ordering) {
        atomic.fetch_or(CLOSED_FLAG, ordering);
    }

    #[inline]
    fn reopen(atomic: &AtomicUsize, ordering: Ordering) {
        atomic.fetch_and(!CLOSED_FLAG, ordering);
    }

    #[inline]
    fn check_overflow(capacity: usize) {
        assert!(
            capacity < usize::MAX >> 2,
            "capacity must be lower than `usize::MAX >> 2`"
        );
    }
}

/// Atomic usize with the following (64bit) representation
/// 64-------------------1--------------0
/// |  dequeuing length  | buffer index |
/// +--------------------+--------------+
/// *buffer index* bit is the index (0 or 1) of the dequeuing buffer
/// *dequeueing length* is the length currently dequeued
#[derive(Copy, Clone)]
struct DequeuingLength(usize);

impl DequeuingLength {
    #[inline]
    fn new(buffer_index: usize, length: usize) -> Self {
        Self(buffer_index | length << 1)
    }

    #[inline]
    fn buffer_index(self) -> usize {
        self.0 & 1
    }

    #[inline]
    fn buffer_len(self) -> usize {
        self.0 >> 1
    }

    #[inline]
    fn try_from_atomic(atomic: usize) -> Result<Self, TryDequeueError> {
        if atomic != DEQUEUING_LOCKED {
            Ok(Self(atomic))
        } else {
            Err(TryDequeueError::Conflict)
        }
    }

    #[inline]
    fn into_atomic(self) -> usize {
        self.0
    }
}

/// A buffered MPSC "swap-buffer" queue.
pub struct Queue<B, N = ()>
where
    B: Buffer,
{
    enqueuing_capacity: CachePadded<AtomicUsize>,
    dequeuing_length: CachePadded<AtomicUsize>,
    buffers: [LoomUnsafeCell<B>; 2],
    buffers_length: [CachePadded<AtomicUsize>; 2],
    capacity: AtomicUsize,
    notify: N,
}

// Needed for `BufferIter`
impl<B, N> AsRef<Queue<B, N>> for Queue<B, N>
where
    B: Buffer,
{
    fn as_ref(&self) -> &Queue<B, N> {
        self
    }
}

// SAFETY: Buffer access is synchronized by the algorithm, but `Send` is required
// because it is owned by the queue
unsafe impl<B, N> Send for Queue<B, N>
where
    B: Buffer + Send,
    N: Send,
{
}
// SAFETY: Buffer access is synchronized by the algorithm, but `Send` is required
// because it is owned by the queue
unsafe impl<B, N> Sync for Queue<B, N>
where
    B: Buffer + Send,
    N: Sync,
{
}

impl<B, N> Queue<B, N>
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
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Queue<VecBuffer<usize>> = Queue::new();
    /// ```
    pub fn new() -> Self {
        let buffers: [LoomUnsafeCell<B>; 2] = Default::default();
        // https://github.com/tokio-rs/loom/issues/277#issuecomment-1633262296
        // SAFETY: exclusive reference to `buffers`
        let capacity = buffers[0].with_mut(|buf| unsafe { &*buf }.capacity());
        EnqueuingCapacity::check_overflow(capacity);
        Self {
            enqueuing_capacity: AtomicUsize::new(EnqueuingCapacity::new(0, capacity).into_atomic())
                .into(),
            dequeuing_length: AtomicUsize::new(DequeuingLength::new(1, 0).into_atomic()).into(),
            buffers,
            buffers_length: Default::default(),
            capacity: AtomicUsize::new(capacity),
            notify: Default::default(),
        }
    }
}

impl<B, N> Queue<B, N>
where
    B: Buffer + Resize,
    N: Default,
{
    /// Creates a new queue with the given capacity.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        EnqueuingCapacity::check_overflow(capacity);
        let buffers: [LoomUnsafeCell<B>; 2] = Default::default();
        // https://github.com/tokio-rs/loom/issues/277#issuecomment-1633262296
        // SAFETY: exclusive reference to `buffers`
        buffers[0].with_mut(|buf| unsafe { &mut *buf }.resize(capacity));
        // SAFETY: exclusive reference to `buffers`
        buffers[1].with_mut(|buf| unsafe { &mut *buf }.resize(capacity));
        Self {
            enqueuing_capacity: AtomicUsize::new(EnqueuingCapacity::new(0, capacity).into_atomic())
                .into(),
            dequeuing_length: AtomicUsize::new(DequeuingLength::new(1, 0).into_atomic()).into(),
            buffers,
            buffers_length: Default::default(),
            capacity: AtomicUsize::new(capacity),
            notify: Default::default(),
        }
    }
}

impl<B, N> Queue<B, N>
where
    B: Buffer,
{
    /// Returns queue's [`Notify`] implementor.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// use swap_buffer_queue::notify::Notify;
    ///
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// queue.notify().notify_dequeue();
    /// ```
    #[inline]
    pub fn notify(&self) -> &N {
        &self.notify
    }

    /// Returns the current enqueuing buffer capacity.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// assert_eq!(queue.capacity(), 42);
    /// ```
    #[inline]
    pub fn capacity(&self) -> usize {
        // cannot use `Buffer::capacity` because of data race
        self.capacity.load(Ordering::Relaxed)
    }

    /// Returns the current enqueuing buffer length.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// assert_eq!(queue.len(), 0);
    /// queue.try_enqueue([0]).unwrap();
    /// assert_eq!(queue.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        let enqueuing =
            EnqueuingCapacity::from_atomic(self.enqueuing_capacity.load(Ordering::Relaxed));
        self.capacity()
            .saturating_sub(enqueuing.remaining_capacity())
    }

    /// Returns `true` if the current enqueuing buffer is empty.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// assert!(queue.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the queue is closed.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// assert!(!queue.is_closed());
    /// queue.close();
    /// assert!(queue.is_closed());
    /// ```
    pub fn is_closed(&self) -> bool {
        EnqueuingCapacity::from_atomic(self.enqueuing_capacity.load(Ordering::Relaxed)).is_closed()
    }

    /// Reopen a closed queue.
    ///
    /// Calling this method when the queue is not closed has no effect.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// queue.close();
    /// assert!(queue.is_closed());
    /// queue.reopen();
    /// assert!(!queue.is_closed());
    /// ```
    pub fn reopen(&self) {
        EnqueuingCapacity::reopen(&self.enqueuing_capacity, Ordering::AcqRel);
    }

    #[inline]
    fn lock_dequeuing(&self) -> Result<DequeuingLength, TryDequeueError> {
        // Protect from concurrent dequeuing by swapping the dequeuing length with a constant
        // marking dequeuing conflict.
        DequeuingLength::try_from_atomic(
            self.dequeuing_length
                .swap(DEQUEUING_LOCKED, Ordering::Relaxed),
        )
    }

    #[allow(clippy::type_complexity)]
    const NO_RESIZE: Option<fn(&mut B) -> (bool, usize)> = None;

    #[inline]
    fn try_dequeue_internal(
        &self,
        dequeuing: DequeuingLength,
        notify_enqueue: impl Fn(),
        resize: Option<impl FnOnce(&mut B) -> (bool, usize)>,
    ) -> Result<BufferSlice<B, N>, TryDequeueError> {
        // If dequeuing length is greater than zero, it means than previous dequeuing is still
        // ongoing, either because previous `try_dequeue` operation returns pending error,
        // or because requeuing (after partial draining for example).
        if let Some(len) = NonZeroUsize::new(dequeuing.buffer_len()) {
            return self
                .try_dequeue_spin(dequeuing.buffer_index(), len)
                .ok_or(TryDequeueError::Pending);
        }
        let next_buffer_index = dequeuing.buffer_index();
        let (resized, inserted_length, next_capa) =
            self.buffers[next_buffer_index].with_mut(|next_buf| {
                // SAFETY: Dequeuing buffer can be accessed mutably
                let next_buffer = unsafe { &mut *next_buf };
                // Resize buffer if needed.
                let (resized, inserted_length) = resize.map_or((false, 0), |f| f(next_buffer));
                (resized, inserted_length, next_buffer.capacity())
            });
        if inserted_length > 0 {
            self.buffers_length[next_buffer_index].fetch_add(inserted_length, Ordering::Relaxed);
        }
        let mut enqueuing =
            EnqueuingCapacity::from_atomic(self.enqueuing_capacity.load(Ordering::Acquire));
        debug_assert_ne!(dequeuing.buffer_index(), enqueuing.buffer_index());
        let capacity =
            // SAFETY: Enqueuing buffer can be immutably accessed.
            self.buffers[enqueuing.buffer_index()].with(|buf| unsafe { &*buf }.capacity());
        // If buffer is empty and has not be resized, return an error (and store back dequeuing)
        if enqueuing.remaining_capacity() == capacity && !resized && inserted_length == 0 {
            self.dequeuing_length
                .store(dequeuing.into_atomic(), Ordering::Relaxed);
            return Err(if enqueuing.is_closed() {
                TryDequeueError::Closed
            } else {
                TryDequeueError::Empty
            });
        }
        // Swap buffers: previous dequeuing buffer become the enqueuing one
        let next_enqueuing = EnqueuingCapacity::new(next_buffer_index, next_capa - inserted_length);
        let mut backoff = 0;
        while let Err(enq) = self.enqueuing_capacity.compare_exchange_weak(
            enqueuing.into_atomic(),
            next_enqueuing.with_closed(enqueuing).into_atomic(),
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            enqueuing = EnqueuingCapacity::from_atomic(enq);
            // Spin in case of concurrent modifications, except when the buffer is full ofc.
            if enqueuing.remaining_capacity() != 0 {
                for _ in 0..1 << backoff {
                    std::hint::spin_loop();
                }
                if backoff < BACKOFF_LIMIT {
                    backoff += 1;
                }
            }
        }
        // Update the queue capacity if needed.
        if self.capacity() != next_capa {
            self.capacity.store(next_capa, Ordering::Relaxed);
        }
        // Notify enqueuers.
        notify_enqueue();
        match NonZeroUsize::new(capacity - enqueuing.remaining_capacity()) {
            // Try to wait ongoing insertions and take ownership of the buffer, then return the
            // buffer slice
            Some(len) => self
                .try_dequeue_spin(enqueuing.buffer_index(), len)
                .ok_or(TryDequeueError::Pending),
            // If the enqueuing buffer was empty, but values has been inserted while resizing,
            // retry.
            None if inserted_length > 0 => self.try_dequeue_internal(
                DequeuingLength::new(enqueuing.buffer_index(), 0),
                notify_enqueue,
                Self::NO_RESIZE,
            ),
            // Otherwise, (empty enqueuing buffer, resized dequeuing one), acknowledge the swap and
            // return empty error
            None => {
                debug_assert!(resized);
                self.dequeuing_length.store(
                    DequeuingLength::new(enqueuing.buffer_index(), 0).into_atomic(),
                    Ordering::Relaxed,
                );
                Err(TryDequeueError::Empty)
            }
        }
    }

    fn try_dequeue_spin(
        &self,
        buffer_index: usize,
        length: NonZeroUsize,
    ) -> Option<BufferSlice<B, N>> {
        for _ in 0..SPIN_LIMIT {
            // Buffers having been swapped, no more enqueuing can happen, we still need to wait
            // for ongoing one. They will be finished when the buffer length (updated after
            // enqueuing) is equal to the expected one.
            // Also, requeuing with potential draining can lead to have an expected length lower
            // than the effective buffer length.
            let buffer_len = self.buffers_length[buffer_index].load(Ordering::Acquire);
            if buffer_len >= length.get() {
                // Returns the slice (range can be shortened by draining + requeuing).
                let range = buffer_len - length.get()..buffer_len;
                let slice = self.buffers[buffer_index]
                    // SAFETY: All enqueuings are done, and buffers having been swapped, this buffer
                    // can now be accessed mutably.
                    // SAFETY: All enqueuing are done, range has been inserted.
                    .with_mut(|buf| unsafe { (*buf).slice(range.clone()) });
                return Some(BufferSlice::new(self, buffer_index, range, slice));
            }
            std::hint::spin_loop();
        }
        // If the enqueuing are still ongoing, just save the dequeuing state in order to retry.
        self.dequeuing_length.store(
            DequeuingLength::new(buffer_index, length.get()).into_atomic(),
            Ordering::Relaxed,
        );
        None
    }

    pub(crate) fn release(&self, buffer_index: usize, range: Range<usize>) {
        // Clears the dequeuing buffer and its length, and release the dequeuing "lock".
        // SAFETY: Dequeued buffer pointed by buffer index can be accessed mutably
        // (see `Queue::try_dequeue_spin`).
        // SAFETY: Range comes from the dequeued slice, so it has been previously inserted.
        self.buffers[buffer_index].with_mut(|buf| unsafe { (*buf).clear(range) });
        self.buffers_length[buffer_index].store(0, Ordering::Release);
        self.dequeuing_length.store(
            DequeuingLength::new(buffer_index, 0).into_atomic(),
            Ordering::Relaxed,
        );
    }

    pub(crate) fn requeue(&self, buffer_index: usize, range: Range<usize>) {
        // Requeuing the buffer just means saving the dequeuing state (or release if there is
        // nothing to requeue).
        let length = range.end - range.start;
        if length > 0 {
            self.dequeuing_length.store(
                DequeuingLength::new(buffer_index, length).into_atomic(),
                Ordering::Relaxed,
            );
        } else {
            self.release(buffer_index, range);
        }
    }
}

impl<B, N> Queue<B, N>
where
    B: Buffer,
    N: Notify,
{
    /// Tries enqueuing the given value into the queue.
    ///
    /// Enqueuing will fail if the queue has insufficient capacity, or if it is closed. In case of
    /// success, it will notify waiting dequeuing operations using [`Notify::notify_dequeue`].
    ///
    /// Enqueuing a zero-sized value is a no-op.
    ///
    /// # Examples
    /// ```
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::TryEnqueueError;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(1);
    /// queue.try_enqueue([0]).unwrap();
    /// // queue is full
    /// assert_eq!(
    ///     queue.try_enqueue([0]),
    ///     Err(TryEnqueueError::InsufficientCapacity([0]))
    /// );
    /// // let's close the queue
    /// queue.close();
    /// assert_eq!(queue.try_enqueue([0]), Err(TryEnqueueError::Closed([0])));
    /// ```
    pub fn try_enqueue<T>(&self, value: T) -> Result<(), TryEnqueueError<T>>
    where
        T: InsertIntoBuffer<B>,
    {
        // Compare-and-swap loop with backoff in order to mitigate contention on the atomic field
        let Some(value_size) = NonZeroUsize::new(value.size()) else {
            return Ok(());
        };
        let mut enqueuing =
            EnqueuingCapacity::from_atomic(self.enqueuing_capacity.load(Ordering::Acquire));
        let mut backoff = None;
        loop {
            // Check if the queue is not closed and try to reserve a slice of the buffer.
            if enqueuing.is_closed() {
                return Err(TryEnqueueError::Closed(value));
            }
            let Some(next_enq) = enqueuing.try_reserve(value_size) else {
                return Err(TryEnqueueError::InsufficientCapacity(value));
            };
            if let Some(ref mut backoff) = backoff {
                for _ in 0..1 << *backoff {
                    std::hint::spin_loop();
                }
                if *backoff < BACKOFF_LIMIT {
                    *backoff += 1;
                }
            }
            match self.enqueuing_capacity.compare_exchange_weak(
                enqueuing.into_atomic(),
                next_enq.into_atomic(),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(enq) => {
                    enqueuing = EnqueuingCapacity::from_atomic(enq);
                    // Spin in case of concurrent modification, except when the buffer index has
                    // modified, which may mean conflict was due to dequeuing.
                    backoff = (next_enq.buffer_index() == enqueuing.buffer_index())
                        .then(|| backoff.unwrap_or(0));
                }
            }
        }
        // Insert the value into the buffer at the index given by subtracting the remaining
        // capacity to the buffer one.
        self.buffers[enqueuing.buffer_index()].with(|buf| {
            // SAFETY: As long as enqueuing is ongoing, i.e. a reserved slice has not been acknowledged
            // in the buffer length (see `BufferWithLength::insert`), buffer cannot be dequeued and can
            // thus be accessed immutably (see `Queue::try_dequeue_spin`).
            let buffer = unsafe { &*buf };
            let index = buffer.capacity() - enqueuing.remaining_capacity();
            // SAFETY: Compare-and-swap makes indexes not overlap, and the buffer is cleared before
            // reusing it for enqueuing (see `Queue::release`).
            unsafe { value.insert_into(buffer, index) };
        });
        self.buffers_length[enqueuing.buffer_index()].fetch_add(value_size.get(), Ordering::AcqRel);
        // Notify dequeuer.
        self.notify.notify_dequeue();
        Ok(())
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
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::TryDequeueError;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// queue.try_enqueue([0]).unwrap();
    /// queue.try_enqueue([1]).unwrap();
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[0, 1]);
    /// // dequeuing cannot be done concurrently (`slice` is still in scope)
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Conflict);
    /// drop(slice);
    /// // let's close the queue
    /// queue.try_enqueue([2]).unwrap();
    /// queue.close();
    /// // queue can be dequeued while closed when not empty
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[2]);
    /// drop(slice);
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Closed)
    /// ```
    pub fn try_dequeue(&self) -> Result<BufferSlice<B, N>, TryDequeueError> {
        self.try_dequeue_internal(
            self.lock_dequeuing()?,
            || self.notify.notify_enqueue(),
            Self::NO_RESIZE,
        )
    }

    /// Closes the queue.
    ///
    /// Closed queue can no more accept enqueuing, but it can be dequeued while not empty.
    /// Calling this method on a closed queue has no effect.
    /// See [`reopen`](Queue::reopen) to reopen a closed queue.
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::{TryDequeueError, TryEnqueueError};
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// queue.try_enqueue([0]).unwrap();
    /// queue.close();
    /// assert!(queue.is_closed());
    /// assert_eq!(queue.try_enqueue([1]), Err(TryEnqueueError::Closed([1])));
    /// assert_eq!(queue.try_dequeue().unwrap().deref(), &[0]);
    /// assert_eq!(queue.try_dequeue().unwrap_err(), TryDequeueError::Closed);
    /// ```
    pub fn close(&self) {
        EnqueuingCapacity::close(&self.enqueuing_capacity, Ordering::AcqRel);
        self.notify.notify_dequeue();
        self.notify.notify_enqueue();
    }
}

impl<B, N> Queue<B, N>
where
    B: Buffer + Resize,
    N: Notify,
{
    /// Tries dequeuing a buffer with all enqueued values from the queue, and resizes the next
    /// buffer to be used for enqueuing.
    ///
    /// This method is an extension of [`try_dequeue`](Queue::try_dequeue) method. In fact,
    /// before swapping the buffers, next one is empty and protected, so it can be resized, and
    /// it is also possible to add values in it before making it available for enqueuing.
    /// This can be used to make the queue [unbounded](Queue#an-amortized-unbounded-recipe).
    ///
    /// It is worth to be noted that only one buffer is resized, so it can lead to asymmetric buffers.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// # use swap_buffer_queue::error::TryEnqueueError;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(1);
    /// queue.try_enqueue([0]).unwrap();
    /// // queue is full
    /// assert_eq!(
    ///     queue.try_enqueue([1]),
    ///     Err(TryEnqueueError::InsufficientCapacity([1]))
    /// );
    /// // dequeue and resize, inserting elements before the buffer is available
    /// let slice = queue
    ///     .try_dequeue_and_resize(3, Some(|| std::iter::once([42])))
    ///     .unwrap();
    /// assert_eq!(slice.deref(), &[0]);
    /// drop(slice);
    /// // capacity has been increased
    /// queue.try_enqueue([1]).unwrap();
    /// queue.try_enqueue([2]).unwrap();
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[42, 1, 2]);
    /// ```
    ///
    /// ## An amortized unbounded recipe
    ///
    /// ```rust
    /// # use std::ops::Deref;
    /// # use std::sync::Mutex;
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::{BufferSlice, InsertIntoBuffer, VecBuffer};
    /// # use swap_buffer_queue::error::{EnqueueError, TryDequeueError, TryEnqueueError};
    /// # use swap_buffer_queue::notify::Notify;
    /// fn enqueue_unbounded<T>(
    ///     queue: &Queue<VecBuffer<T>>,
    ///     overflow: &Mutex<Vec<[T; 1]>>,
    ///     mut value: T,
    /// ) -> Result<(), EnqueueError<[T; 1]>> {
    ///     // first, try to enqueue normally
    ///     match queue.try_enqueue([value]) {
    ///         Err(TryEnqueueError::InsufficientCapacity([v])) => value = v,
    ///         res => return res,
    ///     };
    ///     // if the enqueuing fails, lock the overflow
    ///     let mut guard = overflow.lock().unwrap();
    ///     // retry to enqueue (we never know what happened during lock acquisition)
    ///     match queue.try_enqueue([value]) {
    ///         Err(TryEnqueueError::InsufficientCapacity([v])) => value = v,
    ///         res => return res,
    ///     };
    ///     // then push the values to the overflow vector
    ///     guard.push([value]);
    ///     drop(guard);
    ///     // notify possible waiting dequeue
    ///     queue.notify().notify_dequeue();
    ///     Ok(())
    /// }
    ///
    /// fn try_dequeue_unbounded<'a, T>(
    ///     queue: &'a Queue<VecBuffer<T>>,
    ///     overflow: &Mutex<Vec<[T; 1]>>,
    /// ) -> Result<BufferSlice<'a, VecBuffer<T>, ()>, TryDequeueError> {
    ///     // lock the overflow and use `try_dequeue_and_resize` to drain the overflow into the
    ///     // queue
    ///     let mut guard = overflow.lock().unwrap();
    ///     let vec = &mut guard;
    ///     // `{ vec }` is a trick to get the correct FnOnce inference
    ///     // https://stackoverflow.com/questions/74814588/why-does-rust-infer-fnmut-instead-of-fnonce-for-this-closure-even-though-inferr
    ///     queue.try_dequeue_and_resize(queue.capacity() + vec.len(), Some(|| { vec }.drain(..)))
    /// }
    ///
    /// // queue is initialized with zero capacity
    /// let queue: Queue<VecBuffer<usize>> = Queue::new();
    /// let overflow = Mutex::new(Vec::new());
    /// assert_eq!(queue.capacity(), 0);
    /// enqueue_unbounded(&queue, &overflow, 0).unwrap();
    /// assert_eq!(
    ///     try_dequeue_unbounded(&queue, &overflow).unwrap().deref(),
    ///     &[0]
    /// );
    /// enqueue_unbounded(&queue, &overflow, 1).unwrap();
    /// enqueue_unbounded(&queue, &overflow, 2).unwrap();
    /// assert_eq!(
    ///     try_dequeue_unbounded(&queue, &overflow).unwrap().deref(),
    ///     &[1, 2]
    /// );
    /// ```
    pub fn try_dequeue_and_resize<I>(
        &self,
        capacity: impl Into<Option<usize>>,
        insert: Option<impl FnOnce() -> I>,
    ) -> Result<BufferSlice<B, N>, TryDequeueError>
    where
        I: IntoIterator,
        I::Item: InsertIntoBuffer<B>,
    {
        self.try_dequeue_internal(
            self.lock_dequeuing()?,
            || self.notify.notify_enqueue(),
            Some(move |buffer_mut: &mut B| {
                let resized_capa = capacity
                    .into()
                    .filter(|capa| *capa != buffer_mut.capacity());
                if let Some(capa) = resized_capa {
                    EnqueuingCapacity::check_overflow(capa);
                    buffer_mut.resize(capa);
                }
                let mut length = 0;
                if let Some(insert) = insert {
                    for value in insert() {
                        let Some(value_size) = NonZeroUsize::new(value.size()) else {
                            continue;
                        };
                        if value_size.get() > buffer_mut.capacity() {
                            break;
                        }
                        // SAFETY: Ranges `length..length+value.size()` will obviously not overlap,
                        // and the buffer is cleared before reusing it for enqueuing
                        // (see `Queue::release`)
                        unsafe { value.insert_into(buffer_mut, length) };
                        length += value_size.get();
                    }
                }
                (resized_capa.is_some(), length)
            }),
        )
    }
}

impl<B, N> Queue<B, N>
where
    B: Buffer + Drain,
{
    pub(crate) fn remove(&self, buffer_index: usize, index: usize) -> (B::Value, usize) {
        debug_assert_eq!(
            self.dequeuing_length.load(Ordering::Relaxed),
            DEQUEUING_LOCKED
        );
        // SAFETY: Dequeued buffer pointed by buffer index can be accessed mutably
        // (see `Queue::try_dequeue_spin`).
        // SAFETY: Index comes from an iterator on the dequeued slice, so it has
        // been previously inserted, and can be removed.
        self.buffers[buffer_index].with_mut(|buf| unsafe { (*buf).remove(index) })
    }
}

impl<B, N> Default for Queue<B, N>
where
    B: Buffer,
    N: Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<B, N> fmt::Debug for Queue<B, N>
where
    B: Buffer,
    N: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Queue")
            .field("capacity", &self.capacity())
            .field("len", &self.len())
            .field("notify", &self.notify)
            .finish()
    }
}

impl<B, N> Drop for Queue<B, N>
where
    B: Buffer,
{
    fn drop(&mut self) {
        self.lock_dequeuing()
            .and_then(|deq| self.try_dequeue_internal(deq, || (), Self::NO_RESIZE))
            .ok();
    }
}
