//! [`Buffer`] definition and simple implementations.

use std::{
    cell::UnsafeCell,
    fmt,
    iter::FusedIterator,
    mem,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut, Range},
    panic::{RefUnwindSafe, UnwindSafe},
};

use crossbeam_utils::CachePadded;

use crate::{
    loom::{AtomicUsize, Ordering},
    queue::SBQueue,
};

#[cfg(feature = "buffer")]
#[cfg_attr(docsrs, doc(cfg(feature = "buffer")))]
mod array;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
#[cfg(feature = "buffer")]
#[cfg_attr(docsrs, doc(cfg(feature = "buffer")))]
mod vec;

#[cfg(feature = "buffer")]
pub use array::ArrayBuffer;
#[cfg(feature = "std")]
#[cfg(feature = "buffer")]
pub use vec::VecBuffer;

/// [`SBQueue`] buffer. It is used together with [`BufferValue`].
///
/// # Safety
/// [`Buffer::clear`] *clears* the inserted range (see [`BufferValue::insert_into`]),
/// meaning new values can be inserted.
pub unsafe trait Buffer: Default {
    /// The slice type returned by [`slice`](Buffer::slice) method.
    type Slice<'a>
    where
        Self: 'a;
    /// Returns the buffer's capacity.
    fn capacity(&self) -> usize;
    /// Formats the buffer fields in debugging context.
    fn debug(&self, debug_struct: &mut fmt::DebugStruct);
    /// Returns a slice of the buffer.
    ///
    /// # Safety
    /// Range **must** have been inserted (see [`BufferValue::insert_into`]) before calling
    /// this method.
    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_>;
    /// Clears the buffer.
    ///
    /// # Safety
    /// Range **must** have been inserted (see [`BufferValue::insert_into`]) before calling
    /// this method.
    ///
    /// Calling this method *clears* the inserted value, meaning new values can be inserted.
    unsafe fn clear(&mut self, range: Range<usize>);
}

/// [`Buffer`] value.
///
/// # Safety
/// Range `index..index+value.size()` is considered inserted into the buffer after calling
/// [`BufferValue::insert_into`] (see [`Buffer::slice`]/[`Buffer::clear`])
pub unsafe trait BufferValue<B: Buffer> {
    /// Return the size taken by a value in the buffer.
    fn size(&self) -> usize;
    /// Inserts the value into the buffer at the given index.
    ///
    /// # Safety
    /// For every call to this method, the inserted range `index..index+value.size()` **must not**
    /// overlap with a previously inserted one.
    unsafe fn insert_into(self, buffer: &UnsafeCell<B>, index: usize);
}

/// Resizable [`Buffer`].
pub trait Resize: Buffer {
    /// Resizes the buffer.
    fn resize(&mut self, capacity: usize);
}

/// [`Buffer`] whose values can be drained from.
///
/// # Safety
/// Calling [`Buffer::remove`] decreased the inserted range (see [`BufferValue::insert_into`])
/// by the size of the removed value.
pub unsafe trait Drain: Buffer {
    /// Value to be removed from the buffer
    type Value;
    /// Remove a value from the buffer at a given index and return it with its size.
    ///
    /// # Safety
    /// A value **must** have been inserted at this index (see [`BufferValue::insert_into`])
    /// before calling this method.
    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize);
}

#[derive(Default)]
pub(crate) struct BufferWithLen<B>
where
    B: Buffer,
{
    buffer: UnsafeCell<B>,
    len: CachePadded<AtomicUsize>,
    #[cfg(debug_assertions)]
    removed: AtomicUsize,
}

unsafe impl<B> Send for BufferWithLen<B> where B: Buffer {}
unsafe impl<B> Sync for BufferWithLen<B> where B: Buffer {}
impl<B> UnwindSafe for BufferWithLen<B> where B: Buffer {}
impl<B> RefUnwindSafe for BufferWithLen<B> where B: Buffer {}

impl<B> BufferWithLen<B>
where
    B: Buffer,
{
    pub(crate) fn capacity(&self) -> usize {
        unsafe { &*self.buffer.get() }.capacity()
    }

    pub(crate) fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    pub(crate) fn debug(&self, debug_struct: &mut fmt::DebugStruct) {
        debug_struct.field("len", &self.len());
        unsafe { &*self.buffer.get() }.debug(debug_struct);
    }

    pub(crate) unsafe fn insert<T>(&self, index: usize, value: T) -> bool
    where
        T: BufferValue<B>,
    {
        let size = value.size();
        value.insert_into(&self.buffer, index);
        let prev_len = self.len.fetch_add(size, Ordering::AcqRel);
        prev_len >= index
    }

    pub(crate) unsafe fn slice(&self, range: Range<usize>) -> B::Slice<'_> {
        #[cfg(debug_assertions)]
        debug_assert_eq!(range, self.removed.load(Ordering::Acquire)..self.len());
        (*self.buffer.get()).slice(range)
    }

    pub(crate) unsafe fn clear(&self, range: Range<usize>) {
        #[cfg(debug_assertions)]
        debug_assert_eq!(range, self.removed.load(Ordering::Acquire)..self.len());
        (*self.buffer.get()).clear(range);
        self.len.store(0, Ordering::Release);
        #[cfg(debug_assertions)]
        self.removed.store(0, Ordering::Release);
    }
}

impl<B> BufferWithLen<B>
where
    B: Buffer + Resize,
{
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let mut buf = Self::default();
        if capacity > 0 {
            buf.buffer.get_mut().resize(capacity);
        }
        buf
    }

    pub(crate) unsafe fn resize(&self, capacity: usize) {
        debug_assert_eq!(self.len(), 0);
        if capacity != self.capacity() {
            (*self.buffer.get()).resize(capacity);
        }
    }
}

impl<B> BufferWithLen<B>
where
    B: Buffer + Drain,
{
    pub(crate) unsafe fn remove(&self, index: usize) -> (B::Value, usize) {
        #[cfg(debug_assertions)]
        debug_assert_eq!(index, self.removed.load(Ordering::Acquire));
        #[cfg(debug_assertions)]
        debug_assert!(self.removed.fetch_add(1, Ordering::AcqRel) <= self.len());
        (*self.buffer.get()).remove(index)
    }
}

/// [`Buffer`] slice returned by [`SBQueue::try_dequeue`] (see [`Buffer::Slice`]).
///
/// Buffer is released when the slice is dropped, so the other buffer will be dequeued next,
/// unless [`BufferSlice::Requeue`]/[`BufferSlice::into_iter`] is called.
///
/// # Examples
/// ```
/// # use std::ops::Deref;
/// # use swap_buffer_queue::SBQueue;
/// # use swap_buffer_queue::buffer::VecBuffer;
/// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
/// queue.try_enqueue(0).unwrap();
/// queue.try_enqueue(1).unwrap();
///
/// let slice = queue.try_dequeue().unwrap();
/// assert_eq!(slice.deref(), &[0, 1]);
/// assert_eq!(slice.into_iter().collect::<Vec<_>>(), vec![0, 1]);
/// ```
pub struct BufferSlice<'a, B, N>
where
    B: Buffer,
{
    queue: &'a SBQueue<B, N>,
    buffer_index: usize,
    range: Range<usize>,
    slice: B::Slice<'a>,
}

impl<'a, B, N> BufferSlice<'a, B, N>
where
    B: Buffer,
{
    pub(crate) fn new(
        queue: &'a SBQueue<B, N>,
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
    /// # use swap_buffer_queue::SBQueue;
    /// # use swap_buffer_queue::buffer::VecBuffer;
    /// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
    /// queue.try_enqueue(0).unwrap();
    /// queue.try_enqueue(1).unwrap();
    ///
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[0, 1]);
    /// slice.requeue();
    /// let slice = queue.try_dequeue().unwrap();
    /// assert_eq!(slice.deref(), &[0, 1]);
    /// ```
    pub fn requeue(self) {
        self.queue.requeue(self.buffer_index, self.range.clone());
        mem::forget(self);
    }
}

impl<'a, B, N> fmt::Debug for BufferSlice<'a, B, N>
where
    B: Buffer,
    B::Slice<'a>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.slice, f)
    }
}

impl<'a, B, N> Deref for BufferSlice<'a, B, N>
where
    B: Buffer,
{
    type Target = B::Slice<'a>;

    fn deref(&self) -> &Self::Target {
        &self.slice
    }
}

impl<'a, B, N> DerefMut for BufferSlice<'a, B, N>
where
    B: Buffer,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slice
    }
}

impl<'a, B, N> Drop for BufferSlice<'a, B, N>
where
    B: Buffer,
{
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

    fn into_iter(self) -> Self::IntoIter {
        let slice = ManuallyDrop::new(self);
        BufferIter {
            queue: slice.queue,
            buffer_index: slice.buffer_index,
            range: slice.range.clone(),
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
/// # use swap_buffer_queue::SBQueue;
/// # use swap_buffer_queue::buffer::VecBuffer;
/// let queue: SBQueue<VecBuffer<usize>> = SBQueue::with_capacity(42);
/// queue.try_enqueue(0).unwrap();
/// queue.try_enqueue(1).unwrap();
///
/// let mut iter = queue.try_dequeue().unwrap().into_iter();
/// assert_eq!(iter.next(), Some(0));
/// drop(iter);
/// let mut iter = queue.try_dequeue().unwrap().into_iter();
/// assert_eq!(iter.next(), Some(1));
/// assert_eq!(iter.next(), None);
/// ```
pub struct BufferIter<'a, B, N>
where
    B: Buffer + Drain,
{
    queue: &'a SBQueue<B, N>,
    buffer_index: usize,
    range: Range<usize>,
}

impl<'a, B, N> Drop for BufferIter<'a, B, N>
where
    B: Buffer + Drain,
{
    fn drop(&mut self) {
        self.queue.requeue(self.buffer_index, self.range.clone());
    }
}

impl<'a, B, N> Iterator for BufferIter<'a, B, N>
where
    B: Buffer + Drain,
{
    type Item = B::Value;

    fn next(&mut self) -> Option<Self::Item> {
        if self.range.is_empty() {
            return None;
        }
        let (value, size) = self.queue.remove(self.buffer_index, self.range.start);
        self.range.start += size;
        debug_assert!(self.range.start <= self.range.end);
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, B, N> ExactSizeIterator for BufferIter<'a, B, N> where B: Buffer + Drain {}

impl<'a, B, N> FusedIterator for BufferIter<'a, B, N> where B: Buffer + Drain {}
