//! [`Buffer`] definition and simple implementations.

use std::{
    fmt,
    iter::FusedIterator,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut, Range},
    ptr,
};

use crate::queue::Queue;

mod array;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
mod vec;

pub use array::ArrayBuffer;
#[cfg(feature = "std")]
pub use vec::VecBuffer;

/// [`Queue`] buffer. It is used together with [`InsertIntoBuffer`].
///
/// # Safety
/// [`Buffer::clear`] *clears* the inserted range from the buffer
/// (see [`InsertIntoBuffer::insert_into`]), meaning new values can be inserted.
pub unsafe trait Buffer: Default {
    /// The slice type returned by [`slice`](Buffer::slice) method.
    type Slice<'a>
    where
        Self: 'a;
    /// Returns the buffer's capacity.
    fn capacity(&self) -> usize;
    /// Returns a slice of the buffer.
    ///
    /// # Safety
    /// Range **must** have been inserted (see [`InsertIntoBuffer::insert_into`]) before calling
    /// this method.
    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_>;
    /// Clears the buffer.
    ///
    /// # Safety
    /// Range **must** have been inserted (see [`InsertIntoBuffer::insert_into`]) before calling
    /// this method.
    ///
    /// Calling this method *clears* the inserted value, meaning new values can be inserted.
    unsafe fn clear(&mut self, range: Range<usize>);
}

/// [`Buffer`] value.
///
/// # Safety
/// Range `index..index+value.size()` is considered inserted into the buffer after calling
/// [`InsertIntoBuffer::insert_into`] (see [`Buffer::slice`]/[`Buffer::clear`])
pub unsafe trait InsertIntoBuffer<B: Buffer> {
    /// Returns the size taken by a value in the buffer.
    fn size(&self) -> usize;
    /// Inserts the value into the buffer at the given index.
    ///
    /// # Safety
    /// For every call to this method, the inserted range `index..index+self.size()` **must not**
    /// overlap with a previously inserted one.
    unsafe fn insert_into(self, buffer: &B, index: usize);
}

/// [`Buffer`] kind where value are inserted one by one.
///
/// # Safety
/// `index` is considered inserted into the buffer after calling [`CellBuffer::insert`] (see [`Buffer::slice`]/[`Buffer::clear`])
pub(crate) unsafe trait CellBuffer<T>: Buffer {
    /// Inserts a value into the buffer at the given index.
    ///
    /// # Safety
    /// For every call to this method, `index` **must not** have previously been inserted.
    unsafe fn insert(&self, index: usize, value: T);
}

/// Wrapper to implement [`InsertIntoBuffer`] on iterators.
pub struct ValueIter<I>(pub I);

/// Extension trait to instantiate [`ValueIter`].
pub trait IntoValueIter: Sized {
    /// Iterator type to be wrapped in [`ValueIter`].
    type Iter;
    /// Wrap iterator into [`ValueIter`].
    fn into_value_iter(self) -> ValueIter<Self::Iter>;
}

impl<I> IntoValueIter for I
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
{
    type Iter = I::IntoIter;
    fn into_value_iter(self) -> ValueIter<Self::Iter> {
        ValueIter(self.into_iter())
    }
}

// SAFETY: `insert_into` does initialize the slice in the buffer
unsafe impl<B, I, T> InsertIntoBuffer<B> for ValueIter<I>
where
    B: CellBuffer<T>,
    I: Iterator<Item = T> + ExactSizeIterator,
{
    #[inline]
    fn size(&self) -> usize {
        self.0.len()
    }

    #[inline]
    unsafe fn insert_into(mut self, buffer: &B, index: usize) {
        // don't loop on iterator, because `ExactSizeIterator` is not a sufficient guarantee
        // for unsafe code
        for i in index..(index + self.0.len()) {
            // SAFETY: function contract encompass `CellBuffer::insert` one
            unsafe { buffer.insert(i, self.0.next().unwrap()) };
        }
    }
}

// SAFETY: `insert_into` does initialize the slice in the buffer
unsafe impl<B, T, const N: usize> InsertIntoBuffer<B> for [T; N]
where
    B: CellBuffer<T>,
{
    #[inline]
    fn size(&self) -> usize {
        N
    }

    #[inline]
    unsafe fn insert_into(self, buffer: &B, index: usize) {
        for (i, elt) in self.into_iter().enumerate() {
            // SAFETY: function contract encompass `CellBuffer::insert` one
            unsafe { buffer.insert(index + i, elt) };
        }
    }
}

/// Resizable [`Buffer`].
pub trait Resize: Buffer {
    /// Resizes the buffer.
    fn resize(&mut self, capacity: usize);
}

/// [`Buffer`] whose values can be drained from.
///
/// # Safety
/// Calling [`Drain::remove`] remove the value inserted at index `index
/// (see [`InsertIntoBuffer::insert_into`])
pub unsafe trait Drain: Buffer {
    /// Value to be removed from the buffer
    type Value;
    /// Removes a value from the buffer at a given index and return it.
    ///
    /// # Safety
    /// A value **must** have been inserted at this index (see [`InsertIntoBuffer::insert_into`])
    /// before calling this method.
    unsafe fn remove(&mut self, index: usize) -> Self::Value;
}

/// [`Buffer`] slice returned by [`Queue::try_dequeue`] (see [`Buffer::Slice`]).
///
/// Buffer is released when the slice is dropped, so the other buffer will be dequeued next,
/// unless [`BufferSlice::requeue`]/[`BufferSlice::into_iter`] is called.
///
/// # Examples
/// ```
/// # use std::ops::Deref;
/// # use swap_buffer_queue::Queue;
/// # use swap_buffer_queue::buffer::VecBuffer;
/// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
/// queue.try_enqueue([0]).unwrap();
/// queue.try_enqueue([1]).unwrap();
///
/// let slice = queue.try_dequeue().unwrap();
/// assert_eq!(slice.deref(), &[0, 1]);
/// assert_eq!(slice.into_iter().collect::<Vec<_>>(), vec![0, 1]);
/// ```
pub struct BufferSlice<'a, B, N>
where
    B: Buffer,
{
    queue: &'a Queue<B, N>,
    buffer_index: usize,
    range: Range<usize>,
    slice: B::Slice<'a>,
}

impl<'a, B, N> BufferSlice<'a, B, N>
where
    B: Buffer,
{
    #[inline]
    pub(crate) fn new(
        queue: &'a Queue<B, N>,
        buffer_index: usize,
        range: Range<usize>,
        slice: B::Slice<'a>,
    ) -> Self {
        Self {
            queue,
            buffer_index,
            range,
            slice,
        }
    }

    /// Reinsert the buffer at the beginning queue.
    ///
    /// It will thus de dequeued again next.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
    /// queue.try_enqueue([0]).unwrap();
    /// queue.try_enqueue([1]).unwrap();
    ///
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[0, 1]);
    /// slice.requeue();
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[0, 1]);
    /// ```
    #[inline]
    pub fn requeue(self) {
        let slice = ManuallyDrop::new(self);
        slice.queue.requeue(slice.buffer_index, slice.range.clone());
    }
}

impl<'a, B, N> fmt::Debug for BufferSlice<'a, B, N>
where
    B: Buffer,
    B::Slice<'a>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BufferSlice").field(&self.slice).finish()
    }
}

impl<'a, B, N> Deref for BufferSlice<'a, B, N>
where
    B: Buffer,
{
    type Target = B::Slice<'a>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.slice
    }
}

impl<'a, B, N> DerefMut for BufferSlice<'a, B, N>
where
    B: Buffer,
{
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slice
    }
}

impl<'a, B, N> Drop for BufferSlice<'a, B, N>
where
    B: Buffer,
{
    #[inline]
    fn drop(&mut self) {
        self.queue.release(self.buffer_index, self.range.clone());
    }
}

impl<'a, B, N> IntoIterator for BufferSlice<'a, B, N>
where
    B: Buffer + Drain,
{
    type Item = B::Value;
    type IntoIter = BufferIter<'a, B, N>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        let slice = ManuallyDrop::new(self);
        BufferIter {
            queue: slice.queue,
            buffer_index: slice.buffer_index,
            range: slice.range.clone(),
            _phantom: PhantomData,
        }
    }
}

/// [`Buffer`] iterator returned by [`BufferSlice::into_iter`] (see [`Drain`]).
///
/// Buffer is lazily drained, and requeued (see [`BufferSlice::requeue`]) if the iterator is dropped while non exhausted.
///
/// # Examples
/// ```
/// # use std::ops::Deref;
/// # use swap_buffer_queue::Queue;
/// # use swap_buffer_queue::buffer::VecBuffer;
/// let queue: Queue<VecBuffer<usize>> = Queue::with_capacity(42);
/// queue.try_enqueue([0]).unwrap();
/// queue.try_enqueue([1]).unwrap();
///
/// let mut iter = queue.try_dequeue().unwrap().into_iter();
/// assert_eq!(iter.next(), Some(0));
/// drop(iter);
/// let mut iter = queue.try_dequeue().unwrap().into_iter();
/// assert_eq!(iter.next(), Some(1));
/// assert_eq!(iter.next(), None);
/// ```
pub struct OwnedBufferIter<Q, B, N>
where
    Q: AsRef<Queue<B, N>>,
    B: Buffer,
{
    queue: Q,
    buffer_index: usize,
    range: Range<usize>,
    _phantom: PhantomData<Queue<B, N>>,
}

pub type BufferIter<'a, B, N> = OwnedBufferIter<&'a Queue<B, N>, B, N>;

impl<'a, B, N> BufferIter<'a, B, N>
where
    B: Buffer,
{
    /// Convert back a buffer iterator into a buffer slice.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Arc<Queue<VecBuffer<usize>>> = Arc::new(Queue::with_capacity(42));
    /// queue.try_enqueue([0]).unwrap();
    /// queue.try_enqueue([1]).unwrap();
    ///
    /// let iter = queue.try_dequeue().unwrap().into_iter();
    /// let slice = iter.into_slice();
    /// assert_eq!(slice.deref(), &[0, 1]);
    /// ```
    #[inline]
    pub fn into_slice(self) -> BufferSlice<'a, B, N> {
        let iter = ManuallyDrop::new(self);
        BufferSlice {
            queue: iter.queue,
            buffer_index: iter.buffer_index,
            range: iter.range.clone(),
            slice: iter.queue.get_slice(iter.buffer_index, iter.range.clone()),
        }
    }
}

impl<Q, B, N> OwnedBufferIter<Q, B, N>
where
    Q: AsRef<Queue<B, N>>,
    B: Buffer,
{
    /// Returns a "owned" version of the buffer iterator using a "owned" version of the queue.
    ///
    /// # Examples
    /// ```
    /// # use std::ops::Deref;
    /// # use std::sync::Arc;
    /// # use swap_buffer_queue::Queue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: Arc<Queue<VecBuffer<usize>>> = Arc::new(Queue::with_capacity(42));
    /// queue.try_enqueue([0]).unwrap();
    /// queue.try_enqueue([1]).unwrap();
    ///
    /// let mut iter = queue
    ///     .try_dequeue()
    ///     .unwrap()
    ///     .into_iter()
    ///     .with_owned(queue.clone());
    /// drop(queue); // iter is "owned", queue can be dropped
    /// assert_eq!(iter.next(), Some(0));
    /// assert_eq!(iter.next(), Some(1));
    /// assert_eq!(iter.next(), None);
    /// ```
    #[inline]
    pub fn with_owned<O>(self, queue: O) -> OwnedBufferIter<O, B, N>
    where
        O: AsRef<Queue<B, N>>,
    {
        let iter = ManuallyDrop::new(self);
        assert!(
            ptr::eq(iter.queue.as_ref(), queue.as_ref()),
            "new owner must reference the queue referenced by the iterator"
        );
        OwnedBufferIter {
            queue,
            buffer_index: iter.buffer_index,
            range: iter.range.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<Q, B, N> fmt::Debug for OwnedBufferIter<Q, B, N>
where
    Q: AsRef<Queue<B, N>>,
    B: Buffer,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BufferIter").field(&self.range).finish()
    }
}

impl<Q, B, N> Drop for OwnedBufferIter<Q, B, N>
where
    Q: AsRef<Queue<B, N>>,
    B: Buffer,
{
    #[inline]
    fn drop(&mut self) {
        self.queue
            .as_ref()
            .requeue(self.buffer_index, self.range.clone());
    }
}

impl<Q, B, N> Iterator for OwnedBufferIter<Q, B, N>
where
    Q: AsRef<Queue<B, N>>,
    B: Buffer + Drain,
{
    type Item = B::Value;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.range.is_empty() {
            return None;
        }
        let value = self
            .queue
            .as_ref()
            .remove(self.buffer_index, self.range.start);
        self.range.start += 1;
        Some(value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<Q, B, N> ExactSizeIterator for OwnedBufferIter<Q, B, N>
where
    Q: AsRef<Queue<B, N>>,
    B: Buffer + Drain,
{
}

impl<Q, B, N> FusedIterator for OwnedBufferIter<Q, B, N>
where
    Q: AsRef<Queue<B, N>>,
    B: Buffer + Drain,
{
}
