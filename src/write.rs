//! [`Buffer`](`crate::buffer::Buffer`) implementations to be used with
//! [`Write::write`](std::io::Write::write).
//!
//! [`WriteArrayBuffer`] and [`WriteVecBuffer`] are well suited when there are objects to be
//! serialized with a known-serialization size. Indeed, objects can then be serialized directly on
//! the queue's buffer, avoiding allocation.
//!
//! # Examples
//! ```rust
//! # use std::io::Write;
//! # use swap_buffer_queue::SBQueue;
//! # use swap_buffer_queue::write::{WriteBytesSlice, WriteVecBuffer};
//! // Creates a WriteVecBuffer queue with a 2-bytes header
//! let queue: SBQueue<WriteVecBuffer<2>> = SBQueue::with_capacity((1 << 16) - 1);
//! queue
//!     .try_enqueue((256, |slice: &mut [u8]| { /* write the slice */ }))
//!     .ok()
//!     .unwrap();
//! queue
//!     .try_enqueue((42, |slice: &mut [u8]| { /* write the slice */ }))
//!     .ok()
//!     .unwrap();
//! let mut slice = queue.try_dequeue().unwrap();
//! // Adds a header with the len of the buffer
//! let len = (slice.len() as u16).to_be_bytes();
//! slice.header().copy_from_slice(&len);
//! // Let's pretend we have a writer
//! let mut writer: Vec<u8> = Default::default();
//! assert_eq!(writer.write(slice.frame()).unwrap(), 300);
//! ```

use std::ops::{Deref, DerefMut};

mod array;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
mod vec;

pub use array::WriteArrayBuffer;
#[cfg(feature = "std")]
pub use vec::WriteVecBuffer;

/// A bytes slice with a `HEADER_SIZE`-bytes header and a `TRAILER_SIZE`-bytes trailer.
///
/// It implements [`Deref`] and [`DerefMut`], targeting the *unframed* part of the slice,
/// without the header and the trailer. The complete slice (with header and trailer) can be
/// retrieved using [`frame`](BytesSlice::frame) or [`frame_mut`](BytesSlice::frame_mut) methods.
///
/// # Examples
///
/// ```rust
/// # use std::ops::Deref;
/// # use swap_buffer_queue::buffer::BufferSlice;
/// # use swap_buffer_queue::SBQueue;
/// # use swap_buffer_queue::write::{BytesSlice, WriteBytesSlice, WriteVecBuffer};
/// # let queue: SBQueue<WriteVecBuffer<2, 4>> = SBQueue::with_capacity(42);
/// # queue.try_enqueue((4, |slice: &mut [u8]| slice.copy_from_slice(&[2, 3, 4, 5]))).ok().unwrap();
/// let mut slice: BufferSlice<WriteVecBuffer<2, 4>, _> /* = ... */;
/// # slice = queue.try_dequeue().unwrap();
/// assert_eq!(slice.deref().deref(), &[2, 3, 4, 5]);
/// slice.header().copy_from_slice(&[0, 1]);
/// slice.trailer().copy_from_slice(&[6, 7, 8, 9]);
/// assert_eq!(slice.frame(), &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
/// ```
#[derive(Debug)]
pub struct BytesSlice<'a, const HEADER_SIZE: usize = 0, const TRAILER_SIZE: usize = 0>(
    pub(crate) &'a mut [u8],
);

impl<'a, const HEADER_SIZE: usize, const TRAILER_SIZE: usize>
    BytesSlice<'a, HEADER_SIZE, TRAILER_SIZE>
{
    pub(crate) fn new(slice: &'a mut [u8]) -> Self {
        Self(slice)
    }

    /// Returns a mutable reference on the header part of the slice
    /// (see [examples](BytesSlice#examples)).
    pub fn header(&mut self) -> &mut [u8] {
        &mut self.0[..HEADER_SIZE]
    }

    /// Returns a mutable reference on the trailer part of the slice
    /// (see [examples](BytesSlice#examples)).
    pub fn trailer(&mut self) -> &mut [u8] {
        let len = self.0.len();
        &mut self.0[len - TRAILER_SIZE..]
    }

    /// Returns the complete frame slice, with header and trailer
    /// (see [examples](BytesSlice#examples)).
    pub fn frame(&self) -> &[u8] {
        self.0
    }

    /// Returns the complete mutable frame slice, with header and trailer
    /// (see [examples](BytesSlice#examples)).
    pub fn frame_mut(&mut self) -> &mut [u8] {
        self.0
    }
}

impl<const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Deref
    for BytesSlice<'_, HEADER_SIZE, TRAILER_SIZE>
{
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0[HEADER_SIZE..self.0.len() - TRAILER_SIZE]
    }
}

impl<const HEADER_SIZE: usize, const TRAILER_SIZE: usize> DerefMut
    for BytesSlice<'_, HEADER_SIZE, TRAILER_SIZE>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        let len = self.0.len();
        &mut self.0[HEADER_SIZE..len - TRAILER_SIZE]
    }
}

/// Bytes slice writer, used by [`WriteArrayBuffer`] and [`WriteVecBuffer`].
pub trait WriteBytesSlice {
    /// Returns the size of the slice to be written.
    fn size(&self) -> usize;
    /// Writes the slice.
    fn write(self, slice: &mut [u8]);
}

impl<F> WriteBytesSlice for (usize, F)
where
    F: FnOnce(&mut [u8]),
{
    fn size(&self) -> usize {
        self.0
    }
    fn write(self, slice: &mut [u8]) {
        self.1(slice);
    }
}
