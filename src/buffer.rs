//! [`Buffer`] definition and simple implementations.

use std::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
    panic::{RefUnwindSafe, UnwindSafe},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::queue::SBQueue;

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

/// [`SBQueue`] buffer.
///
/// # Safety
/// This trait is meant to be used inside an [`UnsafeCell`], that's why mutability of `self`
/// parameter doesn't follow safe Rust rules.
/// - [`capacity`](Buffer::capacity) and [`debug`](Buffer::debug) methods must be safe to call
/// concurrently to other methods receiving `&mut self`, e.g. [`insert`](Buffer::insert).
/// - [`insert`](Buffer::insert) method may be called multiple times **concurrently**
///
/// The rest behaves *normally*, i.e. [`slice`](Buffer::slice) method borrow a mutable reference,
/// which must be released to call [`clear`](Buffer::clear), etc.
pub unsafe trait Buffer<T>: Default {
    /// The slice type returned by [`slice`](Buffer::slice) method.
    type Slice<'a>
    where
        Self: 'a;
    /// Return the size taken by a value in the buffer.
    fn value_size(value: &T) -> usize;
    /// Returns the buffer's capacity.
    fn capacity(&self) -> usize;
    /// Formats the buffer fields in debugging context.
    fn debug(&self, debug_struct: &mut fmt::DebugStruct);
    /// Inserts value into the buffer at the given index.
    ///
    /// # Safety
    /// Every call to this method **must** have a non overlapping half-open interval
    /// [`index`,`index+Buffer::value_size(&value)`).
    unsafe fn insert(&mut self, index: usize, value: T);
    /// Returns a slice of the buffer.
    ///
    /// # Safety
    /// Half-open interval [0,`len`) **must** have been inserted (see [`insert`](Buffer::insert))
    /// before calling this method.
    unsafe fn slice(&mut self, len: usize) -> Self::Slice<'_>;
    /// Clears the buffer.
    ///
    /// # Safety
    /// Half-open interval [0,`len`) **must** have been inserted (see [`insert`](Buffer::insert))
    /// before calling this method.
    ///
    /// Calling this method *clears* the inserted value, i.e. inserted interval is reset to [0, 0)
    /// (see [`insert`](Buffer::insert)), meaning new value can be inserted.
    unsafe fn clear(&mut self, len: usize);
}

/// Resizable [`Buffer`].
///
/// # Safety
/// The trait extends [`Buffer`] contract, with [`resize`](Resizable::resize) method following the
/// same rule as [`Buffer::clear`]. It means that [`Buffer::capacity`] can be called concurrently,
/// but not [`Buffer::insert`] or [`Buffer::clear`].
pub unsafe trait Resizable<T>: Buffer<T> {
    /// Resizes the buffer.
    ///
    /// # Safety
    /// This method may be called before any insertion with [`Buffer::insert`]; it must not be
    /// called before [`Buffer::clear`] (or [`Drainable::drain`]).
    unsafe fn resize(&mut self, capacity: usize);
}

/// [`Buffer`] whose value can be removed and returned as an iterator.
///
/// # Safety
/// The trait extends [`Buffer`] contract, with [`drain`](Drainable::drain) method following the
/// same rule as [`Buffer::clear`]. It means that [`Buffer::capacity`] can be called concurrently,
/// but not [`Buffer::insert`] or [`Buffer::clear`].
pub unsafe trait Drainable<T>: Buffer<T> {
    /// The drain type returned by [`drain`](Drainable::drain).
    ///
    /// It may implement [`Iterator<Item=T>'](Iterator).
    type Drain<'a>: Iterator<Item = T>
    where
        Self: 'a;
    /// Removes all elements of the buffer and returns them as an iterator.
    ///
    /// # Safety
    /// Half-open interval [0,`len`) **must** have been inserted (see [`Buffer::insert`]) before
    /// calling this method.
    ///
    /// The iterator returned must be exhausted; it will have the same effect than calling
    /// [`Buffer::clear`]
    unsafe fn drain(&mut self, len: usize) -> Self::Drain<'_>;
}

pub(crate) struct BufferWithLen<B, T>
where
    B: Buffer<T>,
{
    buffer: UnsafeCell<B>,
    len: AtomicUsize,
    _phantom: PhantomData<T>,
}

unsafe impl<B, T> Send for BufferWithLen<B, T> where B: Buffer<T> {}
unsafe impl<B, T> Sync for BufferWithLen<B, T> where B: Buffer<T> {}
impl<B, T> UnwindSafe for BufferWithLen<B, T> where B: Buffer<T> {}
impl<B, T> RefUnwindSafe for BufferWithLen<B, T> where B: Buffer<T> {}

impl<B, T> Default for BufferWithLen<B, T>
where
    B: Buffer<T>,
{
    fn default() -> Self {
        Self {
            buffer: Default::default(),
            len: Default::default(),
            _phantom: Default::default(),
        }
    }
}

impl<B, T> Deref for BufferWithLen<B, T>
where
    B: Buffer<T>,
{
    type Target = B;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.buffer.get() }
    }
}

impl<B, T> BufferWithLen<B, T>
where
    B: Buffer<T>,
{
    pub(crate) fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    pub(crate) fn debug(&self, debug_struct: &mut fmt::DebugStruct) {
        debug_struct.field("len", &self.len());
        unsafe { &*self.buffer.get() }.debug(debug_struct);
    }

    pub(crate) unsafe fn insert(&self, index: usize, value: T) -> bool {
        let size = B::value_size(&value);
        (*self.buffer.get()).insert(index, value);
        let prev_len = self.len.fetch_add(size, Ordering::AcqRel);
        prev_len == index
    }

    pub(crate) unsafe fn slice(&self, len: usize) -> B::Slice<'_> {
        debug_assert_eq!(self.len(), len);
        (*self.buffer.get()).slice(len)
    }

    pub(crate) unsafe fn clear(&self, len: usize) {
        debug_assert_eq!(self.len(), len);
        (*self.buffer.get()).clear(len);
        self.len.store(0, Ordering::Release)
    }
}

impl<B, T> BufferWithLen<B, T>
where
    B: Buffer<T> + Resizable<T>,
{
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let mut buffer = B::default();
        if capacity > 0 {
            unsafe { buffer.resize(capacity) };
        }
        Self {
            buffer: UnsafeCell::new(buffer),
            len: AtomicUsize::new(0),
            _phantom: PhantomData,
        }
    }

    pub(crate) unsafe fn resize(&self, capacity: usize) {
        debug_assert_eq!(self.len(), 0);
        if capacity != self.capacity() {
            (*self.buffer.get()).resize(capacity)
        }
    }
}

impl<B, T> BufferWithLen<B, T>
where
    B: Buffer<T> + Drainable<T>,
{
    pub(crate) unsafe fn drain(&self, len: usize) -> B::Drain<'_> {
        debug_assert_eq!(self.len(), len);
        let drain = (*self.buffer.get()).drain(len);
        self.len.store(0, Ordering::Release);
        drain
    }
}

impl<B, T> Drop for BufferWithLen<B, T>
where
    B: Buffer<T>,
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
/// let queue: SBQueue<VecBuffer<usize>, usize> = SBQueue::with_capacity(42);
/// queue.try_enqueue(0).unwrap();
/// queue.try_enqueue(1).unwrap();
///
/// let slice = queue.try_dequeue().unwrap();
/// assert_eq!(slice.deref(), &[0, 1]);
/// assert_eq!(slice.into_iter().collect::<Vec<_>>(), vec![0, 1]);
/// ```
pub struct BufferSlice<'a, B, T, N>
where
    B: Buffer<T>,
{
    queue: &'a SBQueue<B, T, N>,
    buffer_index: usize,
    len: usize,
    slice: B::Slice<'a>,
}

impl<'a, B, T, N> BufferSlice<'a, B, T, N>
where
    B: Buffer<T>,
{
    pub(crate) fn new(
        queue: &'a SBQueue<B, T, N>,
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

impl<'a, B, T, N> fmt::Debug for BufferSlice<'a, B, T, N>
where
    B: Buffer<T>,
    B::Slice<'a>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.slice, f)
    }
}

impl<'a, B, T, N> Deref for BufferSlice<'a, B, T, N>
where
    B: Buffer<T>,
{
    type Target = B::Slice<'a>;

    fn deref(&self) -> &Self::Target {
        &self.slice
    }
}

impl<'a, B, T, N> DerefMut for BufferSlice<'a, B, T, N>
where
    B: Buffer<T>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slice
    }
}

impl<'a, B, T, N> Drop for BufferSlice<'a, B, T, N>
where
    B: Buffer<T>,
{
    fn drop(&mut self) {
        self.queue.release(self.buffer_index, self.len)
    }
}

impl<'a, B, T, N> IntoIterator for BufferSlice<'a, B, T, N>
where
    B: Buffer<T> + Drainable<T>,
{
    type Item = T;
    type IntoIter = BufferIter<'a, B, T, N>;

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
pub struct BufferIter<'a, B, T, N>
where
    B: Buffer<T> + Drainable<T>,
{
    queue: &'a SBQueue<B, T, N>,
    buffer_index: usize,
    iter: std::iter::Fuse<B::Drain<'a>>,
}

impl<'a, B, T, N> Drop for BufferIter<'a, B, T, N>
where
    B: Buffer<T> + Drainable<T>,
{
    fn drop(&mut self) {
        while self.iter.next().is_some() {}
        self.queue.release(self.buffer_index, 0);
    }
}

impl<'a, B, T, N> Iterator for BufferIter<'a, B, T, N>
where
    B: Buffer<T> + Drainable<T>,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, B, T, N> ExactSizeIterator for BufferIter<'a, B, T, N>
where
    B: Buffer<T> + Drainable<T>,
    B::Drain<'a>: ExactSizeIterator,
{
}
