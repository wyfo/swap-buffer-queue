//! [`Buffer`] definition and simple implementations.

use std::{
    cell::UnsafeCell,
    fmt,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
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
/// This trait is meant to be used inside an [`UnsafeCell`], that's why it doesn't follow safe Rust
/// borrowing rules.
/// - [`capacity`](Buffer::capacity) and [`debug`](Buffer::debug) methods must be safe to call
/// concurrently to other methods receiving mutable reference, e.g. [`clear`](Buffer::clear).
/// - [`BufferValue::insert_into`] may be called multiple times **concurrently**.
///
/// The rest behaves *normally*, i.e. [`slice`](Buffer::slice) method borrow a mutable reference,
/// which must be released to call [`clear`](Buffer::clear), etc.
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
    /// Half-open interval [0,`len`) **must** have been inserted (see [`BufferValue::insert_into`])
    /// before calling this method.
    unsafe fn slice(&mut self, len: usize) -> Self::Slice<'_>;
    /// Clears the buffer.
    ///
    /// # Safety
    /// Half-open interval [0,`len`) **must** have been inserted (see [`BufferValue::insert_into`])
    /// before calling this method.
    ///
    /// Calling this method *clears* the inserted value, i.e. inserted interval is reset to [0, 0),
    /// meaning new value can be inserted.
    unsafe fn clear(&mut self, len: usize);
}

/// [`Buffer`] value.
///
/// # Safety
/// [`BufferValue::insert_into`] may be called multiple times **concurrently** (see
/// [`Buffer` safety section](Buffer#safety).
pub unsafe trait BufferValue<B: Buffer> {
    /// Return the size taken by a value in the buffer.
    fn size(&self) -> usize;
    /// Inserts the value into the buffer at the given index.
    ///
    /// # Safety
    /// Every call to this method **must** have a non overlapping half-open interval
    /// [`index`,`index+Buffer::value_size(&value)`).
    unsafe fn insert_into(self, buffer: &mut B, index: usize);
}

/// Resizable [`Buffer`].
///
/// # Safety
/// The trait extends [`Buffer`] contract, with [`resize`](Resizable::resize) method following the
/// same rule as [`Buffer::clear`]. It means that [`Buffer::capacity`] can be called concurrently,
/// but not [`BufferValue::insert_into`] or [`Buffer::clear`].
pub unsafe trait Resizable: Buffer {
    /// Resizes the buffer.
    ///
    /// # Safety
    /// This method may be called before any insertion with [`BufferValue::insert_into`];
    /// it must not be called before [`Buffer::clear`] (or [`Drainable::drain`]).
    unsafe fn resize(&mut self, capacity: usize);
}

/// [`Buffer`] whose value can be removed and returned as an iterator.
///
/// # Safety
/// The trait extends [`Buffer`] contract, with [`drain`](Drainable::drain) method following the
/// same rule as [`Buffer::clear`]. It means that [`Buffer::capacity`] can be called concurrently,
/// but not [`BufferValue::insert_into`] or [`Buffer::clear`].
pub unsafe trait Drainable: Buffer {
    /// [`Drain`](Drainable::Drain) iterator item type.
    type Item;
    /// The iterator type returned by [`drain`](Drainable::drain).
    type Drain<'a>: Iterator<Item = Self::Item>
    where
        Self: 'a;
    /// Removes all elements of the buffer and returns them as an iterator.
    ///
    /// # Safety
    /// Half-open interval [0,`len`) **must** have been inserted (see [`BufferValue::insert_into`])
    /// before calling this method.
    ///
    /// The iterator returned must be exhausted before calling another mutable method;
    /// it must have the same effect than calling [`Buffer::clear`].
    unsafe fn drain(&mut self, len: usize) -> Self::Drain<'_>;
}

#[derive(Default)]
pub(crate) struct BufferWithLen<B>
where
    B: Buffer,
{
    buffer: UnsafeCell<B>,
    len: CachePadded<AtomicUsize>,
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
        value.insert_into(&mut *self.buffer.get(), index);
        let prev_len = self.len.fetch_add(size, Ordering::AcqRel);
        prev_len >= index
    }

    pub(crate) unsafe fn slice(&self, len: usize) -> B::Slice<'_> {
        debug_assert_eq!(self.len(), len);
        (*self.buffer.get()).slice(len)
    }

    pub(crate) unsafe fn clear(&self, len: usize) {
        debug_assert_eq!(self.len(), len);
        (*self.buffer.get()).clear(len);
        self.len.store(0, Ordering::Release);
    }
}

impl<B> BufferWithLen<B>
where
    B: Buffer + Resizable,
{
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let mut buf = Self::default();
        if capacity > 0 {
            unsafe { buf.buffer.get_mut().resize(capacity) };
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
    B: Buffer + Drainable,
{
    pub(crate) unsafe fn drain(&self, len: usize) -> B::Drain<'_> {
        debug_assert_eq!(self.len(), len);
        let drain = (*self.buffer.get()).drain(len);
        self.len.store(0, Ordering::Release);
        drain
    }
}

impl<B> Drop for BufferWithLen<B>
where
    B: Buffer,
{
    fn drop(&mut self) {
        unsafe { self.clear(self.len()) };
    }
}

/// [`Buffer`] slice returned by [`SBQueue::try_dequeue`] (see [`Buffer::Slice`]).
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
    len: usize,
    slice: B::Slice<'a>,
}

impl<'a, B, N> BufferSlice<'a, B, N>
where
    B: Buffer,
{
    pub(crate) fn new(
        queue: &'a SBQueue<B, N>,
        buffer_index: usize,
        len: usize,
        slice: B::Slice<'a>,
    ) -> Self {
        Self {
            queue,
            buffer_index,
            len,
            slice,
        }
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
        self.queue.release(self.buffer_index, self.len);
    }
}

impl<'a, B, N> IntoIterator for BufferSlice<'a, B, N>
where
    B: Buffer + Drainable,
{
    type Item = B::Item;
    type IntoIter = BufferIter<'a, B, N>;

    fn into_iter(self) -> Self::IntoIter {
        let slice = ManuallyDrop::new(self);
        BufferIter {
            queue: slice.queue,
            buffer_index: slice.buffer_index,
            iter: slice.queue.drain(slice.buffer_index, slice.len).fuse(),
        }
    }
}

/// [`Buffer`] iterator returned by [`BufferSlice::into_iter`] (see [`Drainable::Drain`]).
pub struct BufferIter<'a, B, N>
where
    B: Buffer + Drainable,
{
    queue: &'a SBQueue<B, N>,
    buffer_index: usize,
    iter: std::iter::Fuse<B::Drain<'a>>,
}

impl<'a, B, N> Drop for BufferIter<'a, B, N>
where
    B: Buffer + Drainable,
{
    fn drop(&mut self) {
        while self.iter.next().is_some() {}
        self.queue.release(self.buffer_index, 0);
    }
}

impl<'a, B, N> Iterator for BufferIter<'a, B, N>
where
    B: Buffer + Drainable,
{
    type Item = B::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, B, N> ExactSizeIterator for BufferIter<'a, B, N>
where
    B: Buffer + Drainable,
    B::Drain<'a>: ExactSizeIterator,
{
}
